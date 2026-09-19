use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, TimeZone as _, Utc};
use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::data_gateway::grpc_source::{BoardContinuation, RestoredDragonTigerRequest};
use crate::data_gateway::review::{map_gateway_audit_record, store_gateway_error};
use crate::data_gateway::{DragonTigerStockReview, GatewayBatch, GatewayError};
use crate::database::data_acquisition_audit::{
    append_acquisition_in_transaction, verify_acquisition_receipt_in_transaction,
    DataAcquisitionAuditReceipt,
};
use crate::grpc_client::client::board_attempt::AuthorizedBoardAttempt;
use crate::market_domain::ProviderId;
use crate::monitor::push_job::{raw_digest, IntentId, UtcMicros};
use crate::pipeline::chain_analysis::preparation::{SourceObservation, SourceStatus};

use super::dragon_tiger_codec as codec;
use super::positions::{PositionStageCompletion, ValidatedPositionFacts};
use super::{
    check_lease, inspect_run_on_at_layout, schema, storage, ChainPostCloseError,
    LocalChainPostClose, RunLease, RunRecovery,
};

type Result<T> = std::result::Result<T, ChainPostCloseError>;
type GatewayResult = std::result::Result<GatewayBatch<DragonTigerStockReview>, GatewayError>;

#[derive(Debug, thiserror::Error)]
#[error("dragon-tiger projection failed; original reason retained in protected recovery")]
pub(crate) struct DragonTigerProjectionFailed;

pub(crate) struct DragonTigerProjectionRecovery {
    lhb: HashMap<String, f64>,
    source: SourceObservation,
}

impl DragonTigerProjectionRecovery {
    pub(crate) fn lhb_map(&self) -> &HashMap<String, f64> {
        &self.lhb
    }

    pub(crate) fn source(&self) -> &SourceObservation {
        &self.source
    }
}

pub(crate) struct DragonTigerAttemptRecovery {
    result_bytes: Option<Vec<u8>>,
}

impl DragonTigerAttemptRecovery {
    pub(crate) fn result_bytes(&self) -> Option<&[u8]> {
        self.result_bytes.as_deref()
    }
}

pub(crate) struct DragonTigerRecovery {
    request: codec::RestoredRequest,
    attempts: Vec<DragonTigerAttemptRecovery>,
    final_bytes: Option<Vec<u8>>,
    batch: Option<GatewayBatch<DragonTigerStockReview>>,
    projection: Option<DragonTigerProjectionRecovery>,
    projection_failure_reason: Option<String>,
    receipt: Option<DataAcquisitionAuditReceipt>,
}

impl std::fmt::Debug for DragonTigerRecovery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DragonTigerRecovery")
            .field("request_date", &self.request.date)
            .field("attempt_count", &self.attempts.len())
            .field("complete", &self.is_complete())
            .finish_non_exhaustive()
    }
}

impl DragonTigerRecovery {
    pub(crate) fn is_complete(&self) -> bool {
        self.final_bytes.is_some()
    }
    pub(crate) fn request_date(&self) -> NaiveDate {
        self.request.date
    }
    pub(crate) fn request_observed_at(&self) -> &str {
        &self.request.observed_at
    }
    pub(crate) fn request_bytes(&self) -> &[u8] {
        &self.request.request_wire
    }
    pub(crate) fn retry_policy(&self) -> (u32, u64, u64, u64) {
        self.request.retry_policy
    }
    pub(crate) fn attempts(&self) -> &[DragonTigerAttemptRecovery] {
        &self.attempts
    }
    pub(crate) fn final_bytes(&self) -> Option<&[u8]> {
        self.final_bytes.as_deref()
    }
    pub(crate) fn batch(&self) -> Option<&GatewayBatch<DragonTigerStockReview>> {
        self.batch.as_ref()
    }
    pub(crate) fn projection(&self) -> Option<&DragonTigerProjectionRecovery> {
        self.projection.as_ref()
    }
    pub(crate) fn projection_failure_reason(&self) -> Option<&str> {
        self.projection_failure_reason.as_deref()
    }
    pub(crate) fn audit_receipt(&self) -> Option<&DataAcquisitionAuditReceipt> {
        self.receipt.as_ref()
    }
}

struct Occurrence {
    version: u64,
    request_digest: String,
    acquisition_hash: String,
    request: codec::RestoredRequest,
}

pub(super) struct Call {
    pub(super) occurrence_version: u64,
    attempt: u32,
    begin_version: u64,
    request_digest: String,
    owner: String,
    generation: u64,
}

pub(super) struct Terminal {
    attempt: u32,
    version: u64,
    digest: String,
    request_digest: String,
    bytes: Vec<u8>,
    status: Option<(u64, String)>,
}

impl Terminal {
    pub(super) fn retry_backoff(&self) -> Result<Option<u64>> {
        codec::decode_result(&self.bytes)?.confirmed_retry_backoff()
    }
}

pub(super) struct ErrorMaterial {
    version: u64,
    digest: String,
    error: GatewayError,
    audit: crate::data_gateway::review::OwnedGatewayAuditRecord,
}

pub(super) enum Recovery {
    NeverStarted,
    Planned {
        request: RestoredDragonTigerRequest,
        occurrence_version: u64,
    },
    Retry {
        request: RestoredDragonTigerRequest,
        occurrence_version: u64,
        backoff_ms: u64,
    },
    BegunUnconfirmed,
    Terminal {
        occurrence_version: u64,
        terminal: Terminal,
        projected: GatewayResult,
        material: Option<ErrorMaterial>,
    },
    Complete(codec::Projection),
}

struct BeginRow {
    attempt: u32,
    version: u64,
    request_digest: String,
    owner: String,
    generation: u64,
    begun_at: i64,
}

struct ResultRow {
    attempt: u32,
    begin_version: u64,
    version: u64,
    request_digest: String,
    digest: String,
    bytes: Vec<u8>,
    returned_at: i64,
    committed_at: i64,
}

fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(ChainPostCloseError::SchemaRejected)
    }
}

fn checked_blob(bytes: Vec<u8>, length: i64, digest: &str) -> Result<Vec<u8>> {
    require(
        !bytes.is_empty()
            && usize::try_from(length).ok() == Some(bytes.len())
            && raw_digest(&bytes).as_str() == digest,
    )?;
    Ok(bytes)
}

fn timestamp(now: UtcMicros) -> Result<String> {
    Ok(Utc
        .timestamp_micros(now.get())
        .single()
        .ok_or(ChainPostCloseError::SchemaRejected)?
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn occurrence(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    parent: &PositionStageCompletion,
) -> Result<Option<Occurrence>> {
    let row = transaction
        .query_row(
            "SELECT parent_bytes,parent_length,parent_sha256,request_codec_version,request_bytes,\
                    request_length,request_sha256,acquisition_request_hash,run_version,run_id,\
                    run_context_sha256,input_sha256,lease_owner,lease_generation,planned_at,\
                    parent_kind,positions_run_version,positions_sha256,\
                    position_concept_run_version,position_concept_sha256,\
                    parent_completion_run_version,request_observed_at,\
                    request_local_offset_seconds,request_date,operation,disclosure_limit,\
                    stock_limit,request_id,profile,acquisition_authority,retry_max_attempts,retry_base_delay_ms,\
                    retry_max_delay_ms,retry_jitter_ms \
             FROM chain_post_close_dragon_tiger_occurrences WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, i64>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, i64>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, Option<i64>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, i64>(20)?,
                    row.get::<_, String>(21)?,
                    row.get::<_, i64>(22)?,
                    row.get::<_, String>(23)?,
                    row.get::<_, String>(24)?,
                    row.get::<_, i64>(25)?,
                    row.get::<_, i64>(26)?,
                    row.get::<_, String>(27)?,
                    row.get::<_, String>(28)?,
                    row.get::<_, Option<String>>(29)?,
                    row.get::<_, i64>(30)?,
                    row.get::<_, i64>(31)?,
                    row.get::<_, i64>(32)?,
                    row.get::<_, i64>(33)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("dragon-tiger occurrence"))?;
    let Some(row) = row else { return Ok(None) };
    let parent_bytes = checked_blob(row.0, row.1, &row.2)?;
    let request_bytes = checked_blob(row.4, row.5, &row.6)?;
    let request = codec::decode_request(&request_bytes)?;
    codec::validate_parent_bytes(&parent_bytes, parent)?;
    let version = u64::try_from(row.8).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let generation = u64::try_from(row.13).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    require(
        row.3 == 1
            && row.9 == recovery.context.run_id().as_str()
            && row.10 == recovery.context.canonical_sha256().as_str()
            && row.11 == raw_digest(&recovery.input.encode()?).as_str()
            && generation <= recovery.generation
            && (generation != recovery.generation || row.12 == recovery.owner)
            && version <= recovery.head
            && row.14 <= recovery.updated_at
            && row.15 == parent.kind.as_str()
            && u64::try_from(row.16).ok() == Some(parent.positions_version)
            && row.17 == parent.positions_digest
            && row.18.and_then(|value| u64::try_from(value).ok()) == parent.concepts_version
            && row.19 == parent.concepts_digest
            && u64::try_from(row.20).ok() == Some(parent.completion_version)
            && row.21 == request.observed_at
            && i32::try_from(row.22).ok() == Some(request.local_offset_seconds)
            && row.23 == request.date.format("%Y-%m-%d").to_string()
            && row.24 == "DragonTiger"
            && row.25 == 100
            && row.26 == 5_000
            && row.27 == request.request_id
            && row.28 == "LocalBridgeV1"
            && row.29.as_deref() == request.acquisition_authority.as_deref()
            && u32::try_from(row.30).ok() == Some(request.retry_policy.0)
            && u64::try_from(row.31).ok() == Some(request.retry_policy.1)
            && u64::try_from(row.32).ok() == Some(request.retry_policy.2)
            && u64::try_from(row.33).ok() == Some(request.retry_policy.3)
            && row.7
                == crate::data_gateway::dragon_tiger::dragon_tiger_request_hash(
                    request.date,
                    100,
                    5_000,
                ),
    )?;
    Ok(Some(Occurrence {
        version,
        request_digest: row.6,
        acquisition_hash: row.7,
        request,
    }))
}

fn begins(connection: &Connection, intent: &IntentId) -> Result<Vec<BeginRow>> {
    let mut statement = connection
        .prepare(
            "SELECT attempt_ordinal,run_version,request_sha256,lease_owner,\
                    lease_generation,begun_at FROM chain_post_close_dragon_tiger_attempt_begins \
             WHERE intent_id=?1 ORDER BY attempt_ordinal",
        )
        .map_err(|_| storage("dragon-tiger begins"))?;
    let rows = statement
        .query_map([intent.as_str()], |row| {
            Ok(BeginRow {
                attempt: row.get(0)?,
                version: row.get(1)?,
                request_digest: row.get(2)?,
                owner: row.get(3)?,
                generation: row.get(4)?,
                begun_at: row.get(5)?,
            })
        })
        .map_err(|_| storage("dragon-tiger begins"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("dragon-tiger begins"))?;
    Ok(rows)
}

fn results(
    connection: &Connection,
    intent: &IntentId,
    recovery: &RunRecovery,
) -> Result<Vec<ResultRow>> {
    let mut statement = connection
        .prepare(
            "SELECT attempt_ordinal,begin_run_version,run_version,request_sha256,result_bytes,\
                    result_length,result_sha256,committed_at,returned_at,run_id,\
                    run_context_sha256,input_sha256 \
             FROM chain_post_close_dragon_tiger_attempt_results \
             WHERE intent_id=?1 ORDER BY attempt_ordinal",
        )
        .map_err(|_| storage("dragon-tiger results"))?;
    let rows = statement
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(|_| storage("dragon-tiger results"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("dragon-tiger results"))?;
    let input_digest = raw_digest(&recovery.input.encode()?);
    rows.into_iter()
        .map(|row| {
            require(
                row.9 == recovery.context.run_id().as_str()
                    && row.10 == recovery.context.canonical_sha256().as_str()
                    && row.11 == input_digest.as_str(),
            )?;
            Ok(ResultRow {
                attempt: row.0,
                begin_version: row.1,
                version: row.2,
                request_digest: row.3,
                bytes: checked_blob(row.4, row.5, &row.6)?,
                digest: row.6,
                returned_at: row.8,
                committed_at: row.7,
            })
        })
        .collect()
}

fn status(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    result: &ResultRow,
) -> Result<Option<(u64, String, Option<String>)>> {
    transaction
        .query_row(
            "SELECT run_version,material_bytes,material_length,material_sha256 \
             FROM chain_post_close_dragon_tiger_status_materials \
             WHERE intent_id=?1 AND attempt_ordinal=?2",
            params![intent.as_str(), result.attempt],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("dragon-tiger status"))?
        .map(|row| {
            let bytes = checked_blob(row.1, row.2, &row.3)?;
            Ok((row.0, row.3, codec::decode_status(&bytes)?))
        })
        .transpose()
}

fn restore_terminal_gateway(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    occurrence: &Occurrence,
    result: &ResultRow,
) -> Result<(Terminal, GatewayResult)> {
    let decoded = codec::decode_result(&result.bytes)?;
    require(decoded.confirmed_retry_backoff()?.is_none())?;
    let status_material = status(transaction, intent, result)?;
    require((decoded.terminal_status()?.is_some()) == status_material.is_some())?;
    let gateway = if let Some(response) = decoded.terminal_response()? {
        require(status_material.is_none())?;
        crate::data_gateway::grpc_source::GrpcSource::restore_dragon_tiger_response(
            occurrence.request.profile,
            occurrence.request.acquisition_authority.as_deref(),
            &occurrence.request.request_id,
            response,
        )
    } else {
        let wire = decoded
            .terminal_status()?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let diagnostic = status_material
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .2
            .as_deref();
        Err(
            crate::data_gateway::grpc_source::GrpcSource::restore_dragon_tiger_status(
                occurrence.request.profile,
                &occurrence.request.request_id,
                wire.code,
                &wire.details,
                wire.trailer.as_ref(),
                diagnostic,
            )
            .ok_or(ChainPostCloseError::SchemaRejected)?,
        )
    };
    Ok((
        Terminal {
            attempt: result.attempt,
            version: result.version,
            digest: result.digest.clone(),
            request_digest: result.request_digest.clone(),
            bytes: result.bytes.clone(),
            status: status_material.map(|value| (value.0, value.1)),
        },
        gateway,
    ))
}

fn project_gateway_result(
    gateway: &GatewayResult,
    occurrence: &Occurrence,
) -> Result<codec::Projection> {
    match gateway {
        Ok(GatewayBatch::Available { records, evidence }) => {
            match crate::pipeline::chain_analysis::map_lhb_reviews(records.clone()) {
                Ok(lhb) => Ok(codec::Projection::Available {
                    lhb,
                    source: SourceObservation::from_batch_for_request(
                        SourceStatus::Available,
                        evidence.clone(),
                        occurrence.request.date,
                        occurrence.request.observed_at.clone(),
                    )
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                }),
                Err(reason) => Ok(codec::Projection::Failed(reason)),
            }
        }
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => Ok(codec::Projection::Available {
            lhb: HashMap::new(),
            source: SourceObservation::from_batch_for_request(
                SourceStatus::VerifiedEmpty,
                evidence.clone(),
                occurrence.request.date,
                occurrence.request.observed_at.clone(),
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        }),
        Err(error) => Ok(codec::Projection::Available {
            lhb: HashMap::new(),
            source: SourceObservation::unavailable(error.to_string()).requested(
                occurrence.request.date,
                occurrence.request.observed_at.clone(),
            ),
        }),
    }
}

fn error_material(
    transaction: &Transaction<'_>,
    intent: &IntentId,
) -> Result<Option<ErrorMaterial>> {
    transaction
        .query_row(
            "SELECT run_version,material_bytes,material_length,material_sha256 \
             FROM chain_post_close_dragon_tiger_error_materials WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("dragon-tiger error"))?
        .map(|row| {
            let bytes = checked_blob(row.1, row.2, &row.3)?;
            let (error, audit) = codec::decode_error(&bytes)?;
            Ok(ErrorMaterial {
                version: row.0,
                digest: row.3,
                error,
                audit,
            })
        })
        .transpose()
}

fn receipt(row: &(i64, String, Option<String>, String)) -> DataAcquisitionAuditReceipt {
    DataAcquisitionAuditReceipt {
        audit_id: row.0,
        record_hash: row.1.clone(),
        previous_outcome: row.2.clone(),
        current_outcome: row.3.clone(),
    }
}

fn final_recovery(
    transaction: &Transaction<'_>,
    intent: &IntentId,
) -> Result<Option<(Vec<u8>, codec::Final, DataAcquisitionAuditReceipt)>> {
    transaction
        .query_row(
            "SELECT final_bytes,final_length,final_sha256,audit_id,audit_record_hash,\
                    previous_outcome,current_outcome FROM chain_post_close_dragon_tiger_finals \
             WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("dragon-tiger final"))?
        .map(|row| {
            let bytes = checked_blob(row.0, row.1, &row.2)?;
            let decoded = codec::decode_final(&bytes)?;
            let receipt = receipt(&(row.3, row.4, row.5, row.6));
            Ok((bytes, decoded, receipt))
        })
        .transpose()
}

fn advance(transaction: &Transaction<'_>, lease: &mut RunLease, now: UtcMicros) -> Result<u64> {
    let previous = lease.head;
    let next = previous
        .checked_add(1)
        .ok_or_else(|| storage("dragon-tiger head"))?;
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
        .map_err(|_| storage("dragon-tiger cas"))?;
    if changed != 1 {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    lease.head = next;
    Ok(previous)
}

fn validate_identity(recovery: &RunRecovery, lease: &RunLease) -> Result<()> {
    require(
        recovery.context.run_id() == &lease.run_id
            && recovery.input.encode()? == lease.input.encode()?
            && recovery.generation == lease.generation
            && recovery.head == lease.head,
    )
}

pub(super) fn validate_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<()> {
    validate_facts_at_layout(transaction, intent, run, 10)
}

pub(super) fn validate_facts_at_layout(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
    layout: i64,
) -> Result<()> {
    validate_facts_and_capture_final_at_layout(transaction, intent, run, layout).map(|_| ())
}

pub(super) struct ValidatedDragonTiger<'transaction, 'connection, 'run> {
    transaction: &'transaction Transaction<'connection>,
    run: &'run RunRecovery,
    intent: String,
    final_: Option<ValidatedDragonTigerFinal>,
}

enum PositionValidation<'transaction, 'connection, 'run> {
    Independent,
    Reused(ValidatedPositionFacts<'transaction, 'connection, 'run>),
}

struct ValidatedDragonTigerFinal {
    bytes: Vec<u8>,
    projection: codec::Projection,
}

impl<'transaction, 'connection, 'run> ValidatedDragonTiger<'transaction, 'connection, 'run> {
    fn capture(
        transaction: &'transaction Transaction<'connection>,
        intent: &IntentId,
        run: &'run RunRecovery,
        final_: Option<ValidatedDragonTigerFinal>,
    ) -> Self {
        ValidatedDragonTiger {
            transaction,
            run,
            intent: intent.as_str().to_owned(),
            final_,
        }
    }

    fn matches(&self, transaction: &Transaction<'_>, intent: &IntentId, run: &RunRecovery) -> bool {
        std::ptr::eq(self.transaction, transaction)
            && std::ptr::eq(self.run, run)
            && self.intent == intent.as_str()
    }
}

pub(super) fn validate_facts_and_capture_final_at_layout<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    layout: i64,
) -> Result<ValidatedDragonTiger<'transaction, 'connection, 'run>> {
    validate_facts_and_capture_final_with_position_validation_at_layout(
        transaction,
        intent,
        run,
        layout,
        PositionValidation::Independent,
        None,
    )
}

pub(super) fn validate_facts_and_capture_final_after_position_validation_at_layout<
    'transaction,
    'connection,
    'run,
>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    layout: i64,
    positions: ValidatedPositionFacts<'transaction, 'connection, 'run>,
) -> Result<ValidatedDragonTiger<'transaction, 'connection, 'run>> {
    validate_facts_and_capture_final_after_position_validation_scoped(
        transaction,
        intent,
        run,
        layout,
        positions,
        None,
    )
}

pub(super) fn validate_facts_and_capture_final_after_position_validation_scoped<
    'transaction,
    'connection,
    'run,
>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    layout: i64,
    positions: ValidatedPositionFacts<'transaction, 'connection, 'run>,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<ValidatedDragonTiger<'transaction, 'connection, 'run>> {
    validate_facts_and_capture_final_with_position_validation_at_layout(
        transaction,
        intent,
        run,
        layout,
        PositionValidation::Reused(positions),
        proof,
    )
}

fn validate_facts_and_capture_final_with_position_validation_at_layout<
    'transaction,
    'connection,
    'run,
>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    layout: i64,
    position_validation: PositionValidation<'transaction, 'connection, 'run>,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<ValidatedDragonTiger<'transaction, 'connection, 'run>> {
    if proof.is_some() && layout < 12 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    if layout >= 12 {
        schema::verify_parent_layout_v12_scoped(transaction, proof)?;
    } else {
        require(matches!(layout, 10 | 11))?;
    }
    if let PositionValidation::Reused(positions) = &position_validation {
        require(positions.matches(transaction, intent, run, layout))?;
    }
    let occurrence_count: i64 = transaction
        .query_row(
            "SELECT count(*) FROM chain_post_close_dragon_tiger_occurrences WHERE intent_id=?1",
            [intent.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage("dragon-tiger validation"))?;
    if occurrence_count == 0 {
        let children: i64 = transaction.query_row(
            "SELECT (SELECT count(*) FROM chain_post_close_dragon_tiger_attempt_begins WHERE intent_id=?1)+\
                    (SELECT count(*) FROM chain_post_close_dragon_tiger_attempt_results WHERE intent_id=?1)+\
                    (SELECT count(*) FROM chain_post_close_dragon_tiger_status_materials WHERE intent_id=?1)+\
                    (SELECT count(*) FROM chain_post_close_dragon_tiger_error_materials WHERE intent_id=?1)+\
                    (SELECT count(*) FROM chain_post_close_dragon_tiger_finals WHERE intent_id=?1)",
            [intent.as_str()], |row| row.get(0),
        ).map_err(|_| storage("dragon-tiger validation"))?;
        require(children == 0)?;
        return Ok(ValidatedDragonTiger::capture(
            transaction,
            intent,
            run,
            None,
        ));
    }
    require(occurrence_count == 1)?;
    let parent = match position_validation {
        PositionValidation::Independent => {
            super::positions::inspect_position_stage_completion_at_layout(
                transaction,
                intent,
                run,
                layout,
            )?
        }
        PositionValidation::Reused(positions) => {
            super::positions::position_stage_completion_from_validated_at_layout(
                transaction,
                intent,
                run,
                layout,
                positions,
            )?
        }
    };
    let occurrence = occurrence(transaction, intent, run, &parent)?
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let begins = begins(transaction, intent)?;
    let results = results(transaction, intent, run)?;
    require(results.len() <= begins.len())?;
    for (index, begin) in begins.iter().enumerate() {
        require(
            usize::try_from(begin.attempt).ok() == Some(index + 1)
                && begin.request_digest == occurrence.request_digest
                && begin.version > occurrence.version
                && begin.version <= run.head
                && begin.generation <= run.generation
                && (begin.generation != run.generation || begin.owner == run.owner)
                && begin.begun_at <= run.updated_at,
        )?;
        if let Some(result) = results.get(index) {
            require(
                result.attempt == begin.attempt
                    && result.begin_version == begin.version
                    && result.request_digest == occurrence.request_digest
                    && result.version > begin.version
                    && result.version <= run.head
                    && result.returned_at >= begin.begun_at
                    && result.committed_at >= result.returned_at
                    && result.committed_at <= run.updated_at,
            )?;
            let decoded = codec::decode_result(&result.bytes)?;
            let status = status(transaction, intent, result)?;
            require((decoded.terminal_status()?.is_some()) == status.is_some())?;
        }
    }
    require(results.len() == begins.len() || results.len() + 1 == begins.len())?;
    let final_ = if let Some((bytes, decoded, receipt)) = final_recovery(transaction, intent)? {
        let last = results.last().ok_or(ChainPostCloseError::SchemaRejected)?;
        let (_, expected_gateway) =
            restore_terminal_gateway(transaction, intent, &occurrence, last)?;
        let expected_projection = project_gateway_result(&expected_gateway, &occurrence)?;
        require(codec::final_bytes(&expected_gateway, &expected_projection)? == bytes)?;
        let audit = match &expected_gateway {
            Ok(batch) => map_gateway_audit_record(
                "R-04",
                batch.evidence().provider,
                &occurrence.acquisition_hash,
                &expected_gateway,
                &timestamp(
                    UtcMicros::try_new(run.updated_at)
                        .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                )?,
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?,
            Err(_) => {
                error_material(transaction, intent)?
                    .ok_or(ChainPostCloseError::SchemaRejected)?
                    .audit
            }
        };
        verify_acquisition_receipt_in_transaction(transaction, &receipt, &audit.borrowed("R-04"))
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        Some(ValidatedDragonTigerFinal {
            bytes,
            projection: decoded.projection,
        })
    } else {
        None
    };
    Ok(ValidatedDragonTiger::capture(
        transaction,
        intent,
        run,
        final_,
    ))
}

pub(super) struct MacroParent {
    pub(super) bytes: Vec<u8>,
    pub(super) digest: String,
    pub(super) version: u64,
    pub(super) owner: String,
    pub(super) generation: u64,
    pub(super) applied_at: i64,
    pub(super) projection: codec::Projection,
}

pub(super) fn macro_parent(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<MacroParent> {
    let layout = schema::runtime_layout_version(transaction)?;
    require(matches!(layout, 11 | 12 | 13))?;
    let validated = validate_facts_and_capture_final_at_layout(transaction, intent, run, layout)?;
    macro_parent_from_validated(transaction, intent, run, &validated)
}

pub(super) fn macro_parent_from_validated<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    validated: &ValidatedDragonTiger<'transaction, 'connection, 'run>,
) -> Result<MacroParent> {
    require(validated.matches(transaction, intent, run))?;
    let validated = validated
        .final_
        .as_ref()
        .ok_or(ChainPostCloseError::DragonTigerNotStarted)?;
    let (version, digest, owner, generation, applied_at) = transaction
        .query_row(
            "SELECT run_version,final_sha256,lease_owner,lease_generation,applied_at \
         FROM chain_post_close_dragon_tiger_finals WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|_| storage("macro DragonTiger parent"))?;
    Ok(MacroParent {
        bytes: validated.bytes.clone(),
        digest,
        version,
        owner,
        generation,
        applied_at,
        projection: validated.projection.clone(),
    })
}

impl LocalChainPostClose<'_> {
    fn dragon_tiger_parent(
        transaction: &Transaction<'_>,
        lease: &RunLease,
        now: UtcMicros,
    ) -> Result<(RunRecovery, PositionStageCompletion)> {
        schema::verify_runtime_layout_version(transaction, 10)?;
        let recovery = inspect_run_on_at_layout(transaction, &lease.intent_id, 10)?;
        check_lease(transaction, lease, now)?;
        validate_identity(&recovery, lease)?;
        let parent = super::positions::inspect_position_stage_completion_at_layout(
            transaction,
            &lease.intent_id,
            &recovery,
            10,
        )?;
        Ok((recovery, parent))
    }

    pub(super) fn load_dragon_tiger(
        &mut self,
        lease: &RunLease,
        now: UtcMicros,
    ) -> Result<Recovery> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let (run, parent) = Self::dragon_tiger_parent(&transaction, lease, now)?;
        validate_facts(&transaction, &lease.intent_id, &run)?;
        let occurrence = occurrence(&transaction, &lease.intent_id, &run, &parent)?;
        let begins = begins(&transaction, &lease.intent_id)?;
        let results = results(&transaction, &lease.intent_id, &run)?;
        let final_ = final_recovery(&transaction, &lease.intent_id)?;
        let state = match occurrence {
            None => {
                require(begins.is_empty() && results.is_empty() && final_.is_none())?;
                Recovery::NeverStarted
            }
            Some(occurrence) => {
                if let Some((_, decoded, _)) = final_ {
                    Recovery::Complete(decoded.projection)
                } else if let Some(begin) = begins.last() {
                    require(
                        begin.attempt as usize == begins.len()
                            && begin.request_digest == occurrence.request_digest
                            && begin.version <= run.head
                            && begin.begun_at <= run.updated_at
                            && begin.generation <= run.generation
                            && (begin.generation != run.generation || begin.owner == run.owner),
                    )?;
                    let result = results
                        .iter()
                        .find(|result| result.attempt == begin.attempt);
                    let Some(result) = result else {
                        return Ok(Recovery::BegunUnconfirmed);
                    };
                    require(
                        result.begin_version == begin.version
                            && result.request_digest == occurrence.request_digest
                            && result.committed_at <= run.updated_at,
                    )?;
                    let decoded = codec::decode_result(&result.bytes)?;
                    if let Some(backoff_ms) = decoded.confirmed_retry_backoff()? {
                        Recovery::Retry {
                            request: occurrence.request.resume(begin.attempt + 1),
                            occurrence_version: occurrence.version,
                            backoff_ms,
                        }
                    } else {
                        let (terminal, projected) = restore_terminal_gateway(
                            &transaction,
                            &lease.intent_id,
                            &occurrence,
                            result,
                        )?;
                        Recovery::Terminal {
                            occurrence_version: occurrence.version,
                            terminal,
                            projected,
                            material: error_material(&transaction, &lease.intent_id)?,
                        }
                    }
                } else {
                    require(results.is_empty())?;
                    Recovery::Planned {
                        request: occurrence.request.resume(1),
                        occurrence_version: occurrence.version,
                    }
                }
            }
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(state)
    }

    pub(super) fn begin_dragon_tiger_attempt(
        &mut self,
        mut lease: RunLease,
        observation: Option<DateTime<FixedOffset>>,
        occurrence_version: Option<u64>,
        authorized: &AuthorizedBoardAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, Call)> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, parent) = Self::dragon_tiger_parent(&transaction, &lease, now)?;
        let existing = occurrence(&transaction, &lease.intent_id, &run, &parent)?;
        let (occurrence_version, request_digest) = if let Some(existing) = existing {
            require(observation.is_none() && occurrence_version == Some(existing.version))?;
            (existing.version, existing.request_digest)
        } else {
            let observed = observation.ok_or(ChainPostCloseError::SchemaRejected)?;
            require(occurrence_version.is_none())?;
            let date = observed.date_naive();
            let parent_bytes = codec::parent_bytes(&parent)?;
            let request_bytes = codec::request_bytes(
                date,
                observed.to_rfc3339(),
                observed.offset().local_minus_utc(),
                authorized.request_id(),
                authorized.request_bytes(),
                authorized.profile(),
                authorized.acquisition_authority(),
                authorized.retry_policy(),
            )?;
            let parent_digest = raw_digest(&parent_bytes);
            let request_digest = raw_digest(&request_bytes);
            let acquisition_hash =
                crate::data_gateway::dragon_tiger::dragon_tiger_request_hash(date, 100, 5_000);
            let prior = advance(&transaction, &mut lease, now)?;
            transaction
                .execute(
                    "INSERT INTO chain_post_close_dragon_tiger_occurrences(\
                     intent_id,parent_kind,positions_run_version,positions_sha256,\
                     position_concept_run_version,position_concept_sha256,\
                     parent_completion_run_version,parent_codec_version,parent_bytes,parent_length,\
                     parent_sha256,request_observed_at,request_local_offset_seconds,request_date,\
                     operation,disclosure_limit,stock_limit,request_id,request_codec_version,\
                     request_bytes,request_length,request_sha256,acquisition_request_hash,profile,\
                     acquisition_authority,retry_max_attempts,retry_base_delay_ms,\
                     retry_max_delay_ms,retry_jitter_ms,run_id,run_context_sha256,input_sha256,\
                     lease_owner,lease_generation,prior_head_version,run_version,planned_at) \
                     VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?10,?11,?12,?13,'DragonTiger',\
                     100,5000,?14,1,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,\
                     ?28,?29,?30,?31,?32)",
                    params![
                        lease.intent_id.as_str(),
                        parent.kind.as_str(),
                        parent.positions_version,
                        parent.positions_digest,
                        parent.concepts_version,
                        parent.concepts_digest,
                        parent.completion_version,
                        &parent_bytes,
                        parent_bytes.len(),
                        parent_digest.as_str(),
                        observed.to_rfc3339(),
                        observed.offset().local_minus_utc(),
                        date.format("%Y-%m-%d").to_string(),
                        authorized.request_id(),
                        &request_bytes,
                        request_bytes.len(),
                        request_digest.as_str(),
                        acquisition_hash,
                        authorized.profile(),
                        authorized.acquisition_authority(),
                        authorized.retry_policy().0,
                        authorized.retry_policy().1,
                        authorized.retry_policy().2,
                        authorized.retry_policy().3,
                        lease.run_id.as_str(),
                        run.context.canonical_sha256().as_str(),
                        raw_digest(&run.input.encode()?).as_str(),
                        lease.owner.as_str(),
                        lease.generation,
                        prior,
                        lease.head,
                        now.get(),
                    ],
                )
                .map_err(|_| storage("dragon-tiger occurrence insert"))?;
            (lease.head, request_digest.as_str().to_owned())
        };
        let attempt = authorized.attempt_ordinal();
        let prior = advance(&transaction, &mut lease, now)?;
        let previous = if attempt == 1 {
            None
        } else {
            transaction
                .query_row(
                    "SELECT attempt_ordinal,run_version,result_sha256 \
                     FROM chain_post_close_dragon_tiger_attempt_results \
                     WHERE intent_id=?1 AND attempt_ordinal=?2 AND continuation='Retry'",
                    params![lease.intent_id.as_str(), attempt - 1],
                    |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, u64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage("dragon-tiger previous result"))?
                .ok_or(ChainPostCloseError::SchemaRejected)
                .map(Some)?
        };
        transaction
            .execute(
                "INSERT INTO chain_post_close_dragon_tiger_attempt_begins(\
                 intent_id,attempt_ordinal,occurrence_run_version,request_id,request_sha256,\
                 previous_attempt_ordinal,previous_result_run_version,previous_result_sha256,\
                 run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,\
                 prior_head_version,run_version,begun_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![
                    lease.intent_id.as_str(),
                    attempt,
                    occurrence_version,
                    authorized.request_id(),
                    &request_digest,
                    previous.as_ref().map(|value| value.0),
                    previous.as_ref().map(|value| value.1),
                    previous.as_ref().map(|value| value.2.as_str()),
                    lease.run_id.as_str(),
                    run.context.canonical_sha256().as_str(),
                    raw_digest(&run.input.encode()?).as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    prior,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("dragon-tiger begin insert"))?;
        transaction.commit().map_err(|_| storage("commit"))?;
        let call = Call {
            occurrence_version,
            attempt,
            begin_version: lease.head,
            request_digest,
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
        };
        Ok((lease, call))
    }

    pub(super) fn record_dragon_tiger_result(
        &mut self,
        mut lease: RunLease,
        call: Call,
        completion: &crate::data_gateway::grpc_source::BoardAttemptCompletion,
        now: UtcMicros,
    ) -> Result<(RunLease, Terminal)> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, _) = Self::dragon_tiger_parent(&transaction, &lease, now)?;
        require(call.owner == lease.owner.as_str() && call.generation == lease.generation)?;
        let bytes = codec::result_bytes(completion)?;
        let decoded = codec::decode_result(&bytes)?;
        let digest = raw_digest(&bytes);
        let prior = advance(&transaction, &mut lease, now)?;
        let result_version = lease.head;
        let wire_outcome = if completion.response_bytes.is_some() {
            "Response"
        } else {
            "Status"
        };
        let continuation = match completion.continuation {
            BoardContinuation::Retry { .. } => "Retry",
            BoardContinuation::Terminal => "Terminal",
        };
        transaction
            .execute(
                "INSERT INTO chain_post_close_dragon_tiger_attempt_results(\
             intent_id,attempt_ordinal,begin_run_version,request_sha256,wire_outcome,\
             result_codec_version,result_bytes,result_length,result_sha256,continuation,\
             retry_decision,backoff_ms,run_id,run_context_sha256,input_sha256,lease_owner,\
             lease_generation,prior_head_version,run_version,returned_at,committed_at) \
             VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?19)",
                params![
                    lease.intent_id.as_str(),
                    call.attempt,
                    call.begin_version,
                    &call.request_digest,
                    wire_outcome,
                    &bytes,
                    bytes.len(),
                    digest.as_str(),
                    continuation,
                    decoded.retry_decision(),
                    decoded.backoff_ms(),
                    lease.run_id.as_str(),
                    run.context.canonical_sha256().as_str(),
                    raw_digest(&lease.input.encode()?).as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    prior,
                    lease.head,
                    now.get()
                ],
            )
            .map_err(|_| storage("dragon-tiger result insert"))?;
        let mut status_ref = None;
        if wire_outcome == "Status" {
            let diagnostic = completion
                .processed
                .as_ref()
                .err()
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .safe_diagnostic();
            let material = codec::status_bytes(diagnostic)?;
            let material_digest = raw_digest(&material);
            let status_prior = advance(&transaction, &mut lease, now)?;
            transaction
                .execute(
                    "INSERT INTO chain_post_close_dragon_tiger_status_materials(\
                 intent_id,attempt_ordinal,result_run_version,result_sha256,request_sha256,\
                 provenance,projection_version,material_codec_version,material_bytes,\
                 material_length,material_sha256,run_id,run_context_sha256,input_sha256,\
                 lease_owner,lease_generation,prior_head_version,run_version,captured_at) \
                 VALUES(?1,?2,?3,?4,?5,'Captured',1,1,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                    params![
                        lease.intent_id.as_str(),
                        call.attempt,
                        result_version,
                        digest.as_str(),
                        &call.request_digest,
                        &material,
                        material.len(),
                        material_digest.as_str(),
                        lease.run_id.as_str(),
                        run.context.canonical_sha256().as_str(),
                        raw_digest(&lease.input.encode()?).as_str(),
                        lease.owner.as_str(),
                        lease.generation,
                        status_prior,
                        lease.head,
                        now.get()
                    ],
                )
                .map_err(|_| storage("dragon-tiger status insert"))?;
            status_ref = Some((lease.head, material_digest.as_str().to_owned()));
        }
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((
            lease,
            Terminal {
                attempt: call.attempt,
                version: result_version,
                digest: digest.as_str().to_owned(),
                request_digest: call.request_digest,
                bytes,
                status: status_ref,
            },
        ))
    }

    pub(super) fn confirm_dragon_tiger_error(
        &mut self,
        mut lease: RunLease,
        terminal: &Terminal,
        error: &GatewayError,
        now: UtcMicros,
    ) -> Result<(RunLease, ErrorMaterial)> {
        let observed = timestamp(now)?;
        let stored = store_gateway_error(error);
        let restored =
            crate::data_gateway::dragon_tiger::restore_dragon_tiger_gateway_error(&stored)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        require(store_gateway_error(&restored) == stored)?;
        let projected: GatewayResult = Err(restored);
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, parent) = Self::dragon_tiger_parent(&transaction, &lease, now)?;
        let occurrence = occurrence(&transaction, &lease.intent_id, &run, &parent)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let audit = map_gateway_audit_record(
            "R-04",
            ProviderId::Eastmoney,
            &occurrence.acquisition_hash,
            &projected,
            &observed,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let bytes = codec::error_bytes(stored, audit.clone())?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute(
            "INSERT INTO chain_post_close_dragon_tiger_error_materials(\
             intent_id,terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256,request_sha256,\
             status_material_attempt_ordinal,status_material_run_version,status_material_sha256,\
             material_codec_version,material_bytes,material_length,material_sha256,observed_fallback,\
             run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,captured_at)\
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
            params![lease.intent_id.as_str(),terminal.attempt,terminal.version,&terminal.digest,
                &terminal.request_digest,terminal.status.as_ref().map(|_| terminal.attempt),
                terminal.status.as_ref().map(|value| value.0),
                terminal.status.as_ref().map(|value| value.1.as_str()),&bytes,bytes.len(),&digest,
                &observed,lease.run_id.as_str(),run.context.canonical_sha256().as_str(),
                raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,
                prior,lease.head,now.get()],
        ).map_err(|_| storage("dragon-tiger error insert"))?;
        transaction.commit().map_err(|_| storage("commit"))?;
        let version = lease.head;
        Ok((
            lease,
            ErrorMaterial {
                version,
                digest,
                error: projected.unwrap_err(),
                audit,
            },
        ))
    }

    pub(super) fn finalize_dragon_tiger(
        &mut self,
        mut lease: RunLease,
        occurrence_version: u64,
        terminal: &Terminal,
        gateway: &GatewayResult,
        material: Option<&ErrorMaterial>,
        now: UtcMicros,
    ) -> Result<(RunLease, codec::Projection)> {
        require(gateway.is_ok() == material.is_none())?;
        if let (Err(error), Some(material)) = (gateway, material) {
            require(store_gateway_error(error) == store_gateway_error(&material.error))?;
        }
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, parent) = Self::dragon_tiger_parent(&transaction, &lease, now)?;
        let occurrence = occurrence(&transaction, &lease.intent_id, &run, &parent)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(
            occurrence.version == occurrence_version
                && occurrence.request_digest == terminal.request_digest,
        )?;
        let projection = project_gateway_result(gateway, &occurrence)?;
        let audit = match gateway {
            Ok(batch) => map_gateway_audit_record(
                "R-04",
                batch.evidence().provider,
                &occurrence.acquisition_hash,
                gateway,
                &timestamp(now)?,
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?,
            Err(_) => material
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .audit
                .clone(),
        };
        let bytes = codec::final_bytes(gateway, &projection)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let borrowed = audit.borrowed("R-04");
        let receipt = append_acquisition_in_transaction(&transaction, &borrowed)
            .map_err(|_| storage("dragon-tiger audit append"))?;
        let prior = advance(&transaction, &mut lease, now)?;
        let outcome = match gateway {
            Ok(GatewayBatch::Available { .. }) => "Available",
            Ok(GatewayBatch::VerifiedEmpty(_)) => "VerifiedEmpty",
            Err(_) => "Error",
        };
        transaction.execute(
            "INSERT INTO chain_post_close_dragon_tiger_finals(\
             intent_id,occurrence_run_version,occurrence_request_sha256,terminal_attempt_ordinal,\
             terminal_result_run_version,terminal_result_sha256,error_material_run_version,error_material_sha256,\
             final_outcome,final_codec_version,final_bytes,final_length,final_sha256,audit_id,audit_record_hash,\
             previous_outcome,current_outcome,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,\
             prior_head_version,run_version,applied_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?11,?12,\
             ?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
            params![lease.intent_id.as_str(),occurrence.version,&occurrence.request_digest,terminal.attempt,
                terminal.version,&terminal.digest,material.map(|value| value.version),material.map(|value| value.digest.as_str()),
                outcome,&bytes,bytes.len(),&digest,receipt.audit_id,&receipt.record_hash,&receipt.previous_outcome,
                &receipt.current_outcome,lease.run_id.as_str(),run.context.canonical_sha256().as_str(),
                raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get()],
        ).map_err(|_| storage("dragon-tiger final insert"))?;
        verify_acquisition_receipt_in_transaction(&transaction, &receipt, &borrowed)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, projection))
    }

    pub(crate) fn inspect_dragon_tiger(
        &mut self,
        intent: &IntentId,
    ) -> Result<DragonTigerRecovery> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout_version(&transaction, 10)?;
        let run = inspect_run_on_at_layout(&transaction, intent, 10)?;
        validate_facts(&transaction, intent, &run)?;
        let parent = super::positions::inspect_position_stage_completion_at_layout(
            &transaction,
            intent,
            &run,
            10,
        )?;
        let occurrence = occurrence(&transaction, intent, &run, &parent)?
            .ok_or(ChainPostCloseError::DragonTigerNotStarted)?;
        let attempts = results(&transaction, intent, &run)?
            .into_iter()
            .map(|row| DragonTigerAttemptRecovery {
                result_bytes: Some(row.bytes),
            })
            .collect();
        let final_ = final_recovery(&transaction, intent)?;
        let (final_bytes, batch, projection, projection_failure_reason, receipt) = match final_ {
            None => (None, None, None, None, None),
            Some((bytes, decoded, receipt)) => {
                let batch = decoded.gateway.ok();
                match decoded.projection {
                    codec::Projection::Available { lhb, source } => (
                        Some(bytes),
                        batch,
                        Some(DragonTigerProjectionRecovery { lhb, source }),
                        None,
                        Some(receipt),
                    ),
                    codec::Projection::Failed(reason) => {
                        (Some(bytes), batch, None, Some(reason), Some(receipt))
                    }
                }
            }
        };
        let recovery = DragonTigerRecovery {
            request: occurrence.request,
            attempts,
            final_bytes,
            batch,
            projection,
            projection_failure_reason,
            receipt,
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(recovery)
    }
}
