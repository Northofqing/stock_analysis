//! The positions journal owns its v8 parent identity and seven v9 fact families.
//! Only the shared RPC driver receives the opaque call, terminal and live-B handles.
use super::{
    board_error_codec, check_lease, concept_rpc_codec as codec, inspect_run_on,
    positions::{self, PositionBatch},
    schema, storage, ChainPostCloseError as Error, LocalChainPostClose, RunLease, RunRecovery,
};
use crate::data_gateway::grpc_source::{
    BoardAttemptCompletion, ConnectedBoardQueries, RestoredMembershipRequest,
};
use crate::data_gateway::review::{
    map_gateway_audit_record, restore_gateway_error, store_gateway_error, OwnedGatewayAuditRecord,
};
use crate::data_gateway::{BoardMembershipRecord, GatewayBatch, GatewayError};
use crate::database::data_acquisition_audit::{
    append_acquisition_in_transaction, validate_acquisition_chain_in_transaction,
    verify_acquisition_receipt_in_transaction, DataAcquisitionAuditReceipt,
};
use crate::grpc_client::client::board_attempt::AuthorizedBoardAttempt;
use crate::market_domain::ProviderId;
use crate::monitor::push_job::{raw_digest, IntentId, UtcMicros};
use chrono::{SecondsFormat, TimeZone, Utc};
use rusqlite::{params, types::Value, Connection, Transaction, TransactionBehavior};
use std::collections::{BTreeMap, HashSet};

type Projected = std::result::Result<GatewayBatch<BoardMembershipRecord>, GatewayError>;
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    intent: IntentId,
    run: String,
    cache_version: u64,
    cache_digest: String,
    positions_version: u64,
    positions_digest: String,
    ordinal: u64,
    code: String,
}
impl Scope {
    fn from_batch(batch: &PositionBatch, ordinal: u64, code: &str) -> Result<Self> {
        require(
            batch
                .work
                .iter()
                .any(|work| work.0 == ordinal && work.1 == code),
        )?;
        Ok(Self {
            intent: batch.parent.intent_id.clone(),
            run: batch.parent.run_id.clone(),
            cache_version: batch.parent.cache_version,
            cache_digest: batch.parent.cache_digest.clone(),
            positions_version: batch.parent.positions_version,
            positions_digest: batch.parent.positions_digest.clone(),
            ordinal,
            code: code.to_owned(),
        })
    }
    fn check(&self, batch: &PositionBatch, lease: &RunLease) -> Result<()> {
        require(
            self == &Self::from_batch(batch, self.ordinal, &self.code)?
                && self.intent == lease.intent_id
                && self.run == lease.run_id.as_str(),
        )
    }
}

pub(super) struct Call {
    scope: Scope,
    occurrence: u64,
    attempt: u32,
    begin: u64,
    request_digest: String,
    owner: String,
    generation: u64,
}
impl Call {
    pub(super) fn occurrence_version(&self) -> u64 {
        self.occurrence
    }
}

#[derive(Clone)]
pub(super) struct Terminal {
    scope: Scope,
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
pub(super) struct LiveErrorCapability {
    terminal: Terminal,
    owner: String,
    generation: u64,
    head: u64,
}
pub(super) struct Material {
    scope: Scope,
    version: u64,
    digest: String,
    gateway: GatewayError,
    audit: OwnedGatewayAuditRecord,
}
impl Material {
    pub(super) fn gateway_error(&self) -> GatewayError {
        restore_gateway_error(&store_gateway_error(&self.gateway))
            .expect("validated position error remains restorable")
    }
}
pub(super) struct Projection {
    scope: Scope,
    final_version: u64,
    final_digest: String,
    terminal_version: u64,
    outcome: String,
    raw: String,
}
impl Projection {
    pub(super) fn order(&self) -> u64 {
        self.terminal_version
    }
    pub(super) fn code(&self) -> &str {
        &self.scope.code
    }
    pub(super) fn business_error(&self) -> Option<&str> {
        (self.outcome != "Available").then_some(self.raw.as_str())
    }
}
pub(super) enum Recovery {
    NeverStarted,
    Planned {
        request: RestoredMembershipRequest,
        occurrence_version: u64,
    },
    Retry {
        request: RestoredMembershipRequest,
        occurrence_version: u64,
        backoff_ms: u64,
    },
    BegunUnconfirmed,
    Response {
        terminal: Terminal,
        request: codec::RestoredRequest,
        response: crate::grpc_client::pb::magic::market::v1::QueryResponse,
    },
    Error {
        terminal: Terminal,
        material: Material,
    },
    TerminalUnconfirmed,
    Complete(Projection),
}

fn require(valid: bool) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(Error::SchemaRejected)
    }
}
fn timestamp(now: UtcMicros) -> Result<String> {
    Ok(Utc
        .timestamp_micros(now.get())
        .single()
        .ok_or(Error::SchemaRejected)?
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

// SELECTs and writes stay closed over these seven concrete families. No caller
// supplies connection access, table names, SQL, digest constructors or callbacks.
#[derive(Clone, Copy)]
enum Table {
    Occurrence,
    Begin,
    Result,
    Status,
    Error,
    Final,
    Cache,
}
impl Table {
    fn select(self) -> &'static str {
        match self {
            Self::Occurrence => "SELECT * FROM chain_post_close_position_concept_rpc_occurrences WHERE intent_id=?1 ORDER BY position_ordinal",
            Self::Begin => "SELECT * FROM chain_post_close_position_concept_rpc_attempt_begins WHERE intent_id=?1 ORDER BY position_ordinal,attempt_ordinal",
            Self::Result => "SELECT * FROM chain_post_close_position_concept_rpc_attempt_results WHERE intent_id=?1 ORDER BY position_ordinal,attempt_ordinal",
            Self::Status => "SELECT * FROM chain_post_close_position_concept_rpc_status_materials WHERE intent_id=?1 ORDER BY position_ordinal,attempt_ordinal",
            Self::Error => "SELECT * FROM chain_post_close_position_concept_rpc_error_materials WHERE intent_id=?1 ORDER BY position_ordinal",
            Self::Final => "SELECT * FROM chain_post_close_position_concept_rpc_finals WHERE intent_id=?1 ORDER BY position_ordinal",
            Self::Cache => "SELECT * FROM chain_post_close_position_concept_cache_writes WHERE intent_id=?1 ORDER BY terminal_result_run_version",
        }
    }
    fn time(self) -> &'static str {
        match self {
            Self::Occurrence => "planned_at",
            Self::Begin => "begun_at",
            Self::Result => "committed_at",
            Self::Status | Self::Error => "captured_at",
            Self::Final => "applied_at",
            Self::Cache => "written_at",
        }
    }
}
struct Fact(BTreeMap<String, Value>);
impl Fact {
    fn value(&self, key: &str) -> Result<&Value> {
        self.0.get(key).ok_or(Error::SchemaRejected)
    }
    fn text(&self, key: &str) -> Result<&str> {
        match self.value(key)? {
            Value::Text(value) => Ok(value),
            _ => Err(Error::SchemaRejected),
        }
    }
    fn integer(&self, key: &str) -> Result<i64> {
        match self.value(key)? {
            Value::Integer(value) => Ok(*value),
            _ => Err(Error::SchemaRejected),
        }
    }
    fn number(&self, key: &str) -> Result<u64> {
        u64::try_from(self.integer(key)?).map_err(|_| Error::SchemaRejected)
    }
    fn null(&self, key: &str) -> Result<bool> {
        Ok(matches!(self.value(key)?, Value::Null))
    }
    fn optional_text(&self, key: &str) -> Result<Option<&str>> {
        if self.null(key)? {
            Ok(None)
        } else {
            self.text(key).map(Some)
        }
    }
    fn optional_number(&self, key: &str) -> Result<Option<u64>> {
        if self.null(key)? {
            Ok(None)
        } else {
            self.number(key).map(Some)
        }
    }
    fn blob(&self, prefix: &str) -> Result<&[u8]> {
        let bytes = match self.value(&format!("{prefix}_bytes"))? {
            Value::Blob(value) => value.as_slice(),
            _ => return Err(Error::SchemaRejected),
        };
        require(
            !bytes.is_empty()
                && self.number(&format!("{prefix}_length"))? == bytes.len() as u64
                && self.text(&format!("{prefix}_sha256"))? == raw_digest(bytes).as_str()
                && self.number(&format!("{prefix}_codec_version"))? == 1,
        )?;
        Ok(bytes)
    }
    fn version(&self) -> Result<u64> {
        self.number("run_version")
    }
    fn ordinal(&self) -> Result<u64> {
        self.number("position_ordinal")
    }
    fn parent(
        &self,
        parent: &Fact,
        child_time: &str,
        parent_time: &str,
        same_owner: bool,
    ) -> Result<()> {
        let generation = self.number("lease_generation")?;
        let previous = parent.number("lease_generation")?;
        require(
            self.version()? > parent.version()?
                && self.number("prior_head_version")? >= parent.version()?
                && self.integer(child_time)? >= parent.integer(parent_time)?
                && generation >= previous
                && (!same_owner || generation == previous)
                && (generation != previous
                    || self.text("lease_owner")? == parent.text("lease_owner")?),
        )
    }
}
fn rows(connection: &Connection, intent: &IntentId, table: Table) -> Result<Vec<Fact>> {
    let mut statement = connection
        .prepare(table.select())
        .map_err(|_| storage("position RPC facts"))?;
    let names = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mapped = statement
        .query_map([intent.as_str()], |row| {
            names
                .iter()
                .enumerate()
                .map(|(index, name)| Ok((name.clone(), row.get::<_, Value>(index)?)))
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()
                .map(Fact)
        })
        .map_err(|_| storage("position RPC facts"))?;
    mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("position RPC facts"))
}
struct Facts {
    occurrences: Vec<Fact>,
    begins: Vec<Fact>,
    results: Vec<Fact>,
    statuses: Vec<Fact>,
    errors: Vec<Fact>,
    finals: Vec<Fact>,
    caches: Vec<Fact>,
}
impl Facts {
    fn read(connection: &Connection, intent: &IntentId) -> Result<Self> {
        Ok(Self {
            occurrences: rows(connection, intent, Table::Occurrence)?,
            begins: rows(connection, intent, Table::Begin)?,
            results: rows(connection, intent, Table::Result)?,
            statuses: rows(connection, intent, Table::Status)?,
            errors: rows(connection, intent, Table::Error)?,
            finals: rows(connection, intent, Table::Final)?,
            caches: rows(connection, intent, Table::Cache)?,
        })
    }
}
fn find(rows: &[Fact], ordinal: u64, attempt: Option<u64>) -> Result<Option<&Fact>> {
    let mut found = None;
    for row in rows {
        if row.ordinal()? == ordinal
            && (attempt.is_none() || row.optional_number("attempt_ordinal")? == attempt)
        {
            require(found.is_none())?;
            found = Some(row);
        }
    }
    Ok(found)
}
fn needed(rows: &[Fact], ordinal: u64, attempt: Option<u64>) -> Result<&Fact> {
    find(rows, ordinal, attempt)?.ok_or(Error::SchemaRejected)
}
fn request(occurrence: &Fact) -> Result<codec::RestoredRequest> {
    codec::decode_request(occurrence.blob("request")?)
}
fn terminal(scope: Scope, result: &Fact, facts: &Facts) -> Result<Terminal> {
    let status = find(
        &facts.statuses,
        scope.ordinal,
        Some(result.number("attempt_ordinal")?),
    )?;
    Ok(Terminal {
        scope,
        attempt: u32::try_from(result.number("attempt_ordinal")?)
            .map_err(|_| Error::SchemaRejected)?,
        version: result.version()?,
        digest: result.text("result_sha256")?.to_owned(),
        request_digest: result.text("request_sha256")?.to_owned(),
        bytes: result.blob("result")?.to_vec(),
        status: status
            .map(|row| -> Result<_> {
                Ok((row.version()?, row.text("material_sha256")?.to_owned()))
            })
            .transpose()?,
    })
}
fn material(scope: Scope, row: &Fact) -> Result<Material> {
    let (gateway, audit) = board_error_codec::decode_error(row.blob("material")?)?;
    Ok(Material {
        scope,
        version: row.version()?,
        digest: row.text("material_sha256")?.to_owned(),
        gateway: restore_gateway_error(&gateway).map_err(|_| Error::SchemaRejected)?,
        audit,
    })
}
fn projection(scope: Scope, row: &Fact) -> Result<Projection> {
    let (outcome, raw) = codec::decode_final(row.blob("final")?)?;
    Ok(Projection {
        scope,
        final_version: row.version()?,
        final_digest: row.text("final_sha256")?.to_owned(),
        terminal_version: row.number("terminal_result_run_version")?,
        outcome,
        raw,
    })
}
fn restored(request: codec::RestoredRequest, attempt: u32) -> RestoredMembershipRequest {
    RestoredMembershipRequest::new(
        request.code,
        request.request_id,
        request.request,
        request.profile,
        request.acquisition_authority,
        request.retry_policy,
        attempt,
    )
}

fn validate_cache_history_barrier(
    facts: &Facts,
    batch: &PositionBatch,
    cache: &Fact,
) -> Result<()> {
    require(facts.finals.len() == batch.work.len() && facts.occurrences.len() == batch.work.len())?;
    for final_ in &facts.finals {
        cache.parent(final_, "written_at", "applied_at", false)?;
    }
    Ok(())
}

fn validate_rows(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    facts: &Facts,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
    validated_positions: Option<&positions::ValidatedPositionFacts<'_, '_, '_>>,
) -> Result<()> {
    if proof.is_some() && layout_version < 12 {
        return Err(Error::SchemaRejected);
    }
    if layout_version >= 12 {
        schema::verify_parent_layout_v12_scoped(transaction, proof)?;
    } else if !matches!(layout_version, 9 | 10 | 11) {
        return Err(Error::UnsupportedVersion);
    }
    if let Some(positions) = validated_positions {
        require(
            layout_version >= 12
                && proof.is_some()
                && positions.matches(transaction, intent, recovery, layout_version),
        )?;
    }
    if fact_families(facts).iter().all(|(_, rows)| rows.is_empty()) {
        return Ok(());
    }
    let material = match validated_positions {
        Some(positions) => positions.position_batch_parent_for_read_pass(
            transaction,
            intent,
            recovery,
            layout_version,
        )?,
        None => positions::load_position_batch_parent_scoped(
            transaction,
            intent,
            recovery,
            layout_version,
            proof,
        )?,
    };
    let batch = material
        .ok_or(Error::SchemaRejected)?
        .into_batch()
        .map_err(|_| Error::SchemaRejected)?;
    validate_rows_with_batch(
        transaction,
        intent,
        recovery,
        facts,
        layout_version,
        &batch,
        proof,
    )
}

fn fact_families(facts: &Facts) -> [(Table, &[Fact]); 7] {
    [
        (Table::Occurrence, facts.occurrences.as_slice()),
        (Table::Begin, facts.begins.as_slice()),
        (Table::Result, facts.results.as_slice()),
        (Table::Status, facts.statuses.as_slice()),
        (Table::Error, facts.errors.as_slice()),
        (Table::Final, facts.finals.as_slice()),
        (Table::Cache, facts.caches.as_slice()),
    ]
}

fn validate_rows_with_batch(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    facts: &Facts,
    layout_version: i64,
    batch: &PositionBatch,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<()> {
    if proof.is_some() && layout_version < 12 {
        return Err(Error::SchemaRejected);
    }
    if layout_version >= 12 {
        schema::verify_parent_layout_v12_scoped(transaction, proof)?;
    } else if !matches!(layout_version, 9 | 10 | 11) {
        return Err(Error::UnsupportedVersion);
    }
    let families = fact_families(facts);
    if families.iter().all(|(_, rows)| rows.is_empty()) {
        return Ok(());
    }
    let parent = &batch.parent;
    let mut versions = HashSet::new();
    for (table, rows) in families {
        for fact in rows {
            require(
                fact.text("intent_id")? == intent.as_str()
                    && fact.number("cache_material_run_version")? == parent.cache_version
                    && fact.text("run_id")? == parent.run_id
                    && fact.text("run_context_sha256")? == parent.context_digest
                    && fact.text("input_sha256")? == parent.input_digest
                    && fact.number("lease_generation")? >= 1
                    && fact.number("lease_generation")? <= recovery.generation
                    && (fact.number("lease_generation")? != recovery.generation
                        || fact.text("lease_owner")? == recovery.owner)
                    && !fact.text("lease_owner")?.is_empty()
                    && fact.number("prior_head_version")?.checked_add(1) == Some(fact.version()?)
                    && fact.version()? <= recovery.head
                    && versions.insert(fact.version()?)
                    && fact.integer(table.time())? >= 0
                    && fact.integer(table.time())? <= recovery.updated_at
                    && batch
                        .work
                        .iter()
                        .any(|(ordinal, _)| Some(*ordinal) == fact.ordinal().ok()),
            )?;
        }
    }
    let mut ordinals = HashSet::new();
    for occurrence in &facts.occurrences {
        let ordinal = occurrence.ordinal()?;
        let decoded = request(occurrence)?;
        let scope = Scope::from_batch(&batch, ordinal, occurrence.text("code")?)?;
        require(
            ordinals.insert(ordinal)
                && occurrence.text("cache_material_sha256")? == parent.cache_digest
                && occurrence.number("positions_run_version")? == parent.positions_version
                && occurrence.text("positions_sha256")? == parent.positions_digest
                && occurrence.number("prior_head_version")? >= parent.cache_version
                && occurrence.text("operation")? == "BoardConstituents"
                && decoded.code == scope.code
                && decoded.request_id == occurrence.text("request_id")?
                && occurrence.text("profile")? == "LocalBridgeV1"
                && decoded.acquisition_authority.as_deref()
                    == occurrence.optional_text("acquisition_authority")?
                && decoded.retry_policy
                    == (
                        u32::try_from(occurrence.number("retry_max_attempts")?)
                            .map_err(|_| Error::SchemaRejected)?,
                        occurrence.number("retry_base_delay_ms")?,
                        occurrence.number("retry_max_delay_ms")?,
                        occurrence.number("retry_jitter_ms")?,
                    )
                && occurrence.text("acquisition_request_hash")?
                    == crate::data_gateway::review::board_membership_request_hash(&scope.code),
        )?;
        let cache_parent = transaction.query_row(
            "SELECT committed_at,lease_generation,lease_owner FROM chain_post_close_position_concept_materials WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(), parent.cache_version],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?, row.get::<_, String>(2)?))
        ).map_err(|_| storage("position RPC parent envelope"))?;
        require(
            occurrence.integer("planned_at")? >= cache_parent.0
                && occurrence.number("lease_generation")? >= cache_parent.1
                && (occurrence.number("lease_generation")? != cache_parent.1
                    || occurrence.text("lease_owner")? == cache_parent.2),
        )?;
        let mut next_attempt = 1;
        for begin in &facts.begins {
            if begin.ordinal()? != ordinal {
                continue;
            }
            require(
                begin.number("attempt_ordinal")? == next_attempt
                    && next_attempt <= u64::from(decoded.retry_policy.0)
                    && begin.number("occurrence_run_version")? == occurrence.version()?
                    && begin.text("request_id")? == decoded.request_id
                    && begin.text("request_sha256")? == occurrence.text("request_sha256")?,
            )?;
            begin.parent(occurrence, "begun_at", "planned_at", false)?;
            if next_attempt == 1 {
                require(
                    begin.null("previous_attempt_ordinal")?
                        && begin.null("previous_result_run_version")?
                        && begin.null("previous_result_sha256")?,
                )?;
            } else {
                let previous = needed(&facts.results, ordinal, Some(next_attempt - 1))?;
                require(
                    previous.text("continuation")? == "Retry"
                        && begin.number("previous_attempt_ordinal")? == next_attempt - 1
                        && begin.number("previous_result_run_version")? == previous.version()?
                        && begin.text("previous_result_sha256")?
                            == previous.text("result_sha256")?,
                )?;
                begin.parent(previous, "begun_at", "committed_at", false)?;
            }
            next_attempt += 1;
        }
    }
    for begin in &facts.begins {
        needed(&facts.occurrences, begin.ordinal()?, None)?;
    }
    for result in &facts.results {
        let ordinal = result.ordinal()?;
        let attempt = result.number("attempt_ordinal")?;
        let occurrence = needed(&facts.occurrences, ordinal, None)?;
        let begin = needed(&facts.begins, ordinal, Some(attempt))?;
        let decoded = codec::decode_result(result.blob("result")?)?;
        require(
            result.number("begin_run_version")? == begin.version()?
                && result.text("request_sha256")? == occurrence.text("request_sha256")?
                && result.integer("returned_at")? >= begin.integer("begun_at")?
                && result.integer("committed_at")? >= result.integer("returned_at")?
                && result.text("retry_decision")? == decoded.retry_decision()
                && result.optional_number("backoff_ms")? == decoded.backoff_ms(),
        )?;
        result.parent(begin, "returned_at", "begun_at", true)?;
        let status = find(&facts.statuses, ordinal, Some(attempt))?;
        if decoded.terminal_response()?.is_some() {
            require(
                result.text("wire_outcome")? == "Response"
                    && result.text("continuation")? == "Terminal"
                    && result.text("retry_decision")? == "NoRetry"
                    && status.is_none(),
            )?;
        } else {
            let status = status.ok_or(Error::SchemaRejected)?;
            let diagnostic = board_error_codec::decode_status(status.blob("material")?)?;
            let wire = decoded.status_wire()?.ok_or(Error::SchemaRejected)?;
            let restored_request = request(occurrence)?;
            let method = crate::grpc_contract::methods::MethodIdentity::from_client_operation(
                restored_request.profile,
                crate::grpc_client::pb::magic::market::v1::Operation::BoardConstituents,
            )
            .map_err(|_| Error::SchemaRejected)?;
            let transport_error = crate::grpc_client::errors::restore_persisted_status_error(
                wire.code,
                &wire.details,
                wire.trailer.as_ref(),
                diagnostic.as_deref(),
                crate::grpc_client::errors::StatusErrorContext::data(
                    method,
                    &restored_request.request_id,
                ),
            )
            .ok_or(Error::SchemaRejected)?;
            let decision = crate::grpc_client::retry::retry_decision(&transport_error);
            let expected_decision = match decision {
                crate::grpc_client::retry::RetryDecision::RetryBackoff => "RetryBackoff",
                crate::grpc_client::retry::RetryDecision::RetryBounded => "RetryBounded",
                crate::grpc_client::retry::RetryDecision::NoRetry => "NoRetry",
            };
            let (max_attempts, base_delay_ms, max_delay_ms, jitter_ms) =
                restored_request.retry_policy;
            let policy = crate::grpc_client::retry::RetryPolicy {
                max_attempts,
                base_delay_ms,
                max_delay_ms,
                jitter_ms,
            };
            require(
                result.text("wire_outcome")? == "Status"
                    && decoded.retry_decision() == expected_decision
                    && (1..=u64::from(max_attempts)).contains(&attempt),
            )?;
            let should_retry = decision != crate::grpc_client::retry::RetryDecision::NoRetry
                && attempt < u64::from(max_attempts);
            if should_retry {
                let backoff = decoded
                    .confirmed_retry_backoff()?
                    .ok_or(Error::SchemaRejected)?;
                require(
                    result.text("continuation")? == "Retry"
                        && u128::from(backoff)
                            == policy
                                .backoff(u32::try_from(attempt).map_err(|_| Error::SchemaRejected)?)
                                .as_millis(),
                )?;
            } else {
                // Non-retryable Status stops immediately; a retryable Status only
                // terminates at the frozen attempt limit, preserving its decision.
                require(
                    result.text("continuation")? == "Terminal"
                        && decoded.terminal_status()?.is_some()
                        && decoded.backoff_ms().is_none(),
                )?;
            }
        }
        if result.text("continuation")? == "Terminal" {
            require(!facts.begins.iter().any(|later| {
                later.ordinal().ok() == Some(ordinal)
                    && later
                        .number("attempt_ordinal")
                        .ok()
                        .is_some_and(|value| value > attempt)
            }))?;
        }
    }
    for status in &facts.statuses {
        let result = needed(
            &facts.results,
            status.ordinal()?,
            Some(status.number("attempt_ordinal")?),
        )?;
        require(
            result.text("wire_outcome")? == "Status"
                && status.number("result_run_version")? == result.version()?
                && status.text("result_sha256")? == result.text("result_sha256")?
                && status.text("request_sha256")? == result.text("request_sha256")?
                && status.text("provenance")? == "Captured"
                && status.number("projection_version")? == 1
                && status.number("prior_head_version")? == result.version()?,
        )?;
        status.parent(result, "captured_at", "committed_at", true)?;
        board_error_codec::decode_status(status.blob("material")?)?;
    }
    for error in &facts.errors {
        let ordinal = error.ordinal()?;
        let occurrence = needed(&facts.occurrences, ordinal, None)?;
        let result = needed(
            &facts.results,
            ordinal,
            Some(error.number("terminal_attempt_ordinal")?),
        )?;
        require(
            result.text("continuation")? == "Terminal"
                && error.number("terminal_result_run_version")? == result.version()?
                && error.text("terminal_result_sha256")? == result.text("result_sha256")?
                && error.text("request_sha256")? == occurrence.text("request_sha256")?,
        )?;
        error.parent(result, "captured_at", "committed_at", true)?;
        let decoded = codec::decode_result(result.blob("result")?)?;
        let restored_request = request(occurrence)?;
        let projected = if result.text("wire_outcome")? == "Status" {
            let status = needed(
                &facts.statuses,
                ordinal,
                Some(result.number("attempt_ordinal")?),
            )?;
            require(
                error.number("status_material_attempt_ordinal")?
                    == result.number("attempt_ordinal")?
                    && error.number("status_material_run_version")? == status.version()?
                    && error.text("status_material_sha256")? == status.text("material_sha256")?
                    && error.number("prior_head_version")? == status.version()?,
            )?;
            error.parent(status, "captured_at", "captured_at", true)?;
            let wire = decoded.terminal_status()?.ok_or(Error::SchemaRejected)?;
            let diagnostic = board_error_codec::decode_status(status.blob("material")?)?;
            Err(ConnectedBoardQueries::restore_memberships_status(
                restored_request.profile,
                &restored_request.request_id,
                wire.code,
                &wire.details,
                wire.trailer.as_ref(),
                diagnostic.as_deref(),
            )
            .ok_or(Error::SchemaRejected)?)
        } else {
            require(
                error.null("status_material_attempt_ordinal")?
                    && error.null("status_material_run_version")?
                    && error.null("status_material_sha256")?
                    && error.number("prior_head_version")? == result.version()?,
            )?;
            ConnectedBoardQueries::restore_memberships_response(
                restored_request.profile,
                restored_request.acquisition_authority.as_deref(),
                &restored_request.request_id,
                decoded.terminal_response()?.ok_or(Error::SchemaRejected)?,
            )
        };
        let (stored_error, stored_audit) =
            board_error_codec::decode_error(error.blob("material")?)?;
        let projected_error = projected.as_ref().err().ok_or(Error::SchemaRejected)?;
        require(
            store_gateway_error(projected_error) == stored_error
                && restore_gateway_error(&stored_error).is_ok()
                && map_gateway_audit_record(
                    "board-memberships",
                    ProviderId::Tdx,
                    occurrence.text("acquisition_request_hash")?,
                    &projected,
                    error.text("observed_fallback")?,
                )
                .map_err(|_| Error::SchemaRejected)?
                    == stored_audit
                && error.text("observed_fallback")?
                    == timestamp(
                        UtcMicros::try_new(error.integer("captured_at")?)
                            .map_err(|_| Error::SchemaRejected)?,
                    )?,
        )?;
    }
    for final_ in &facts.finals {
        let ordinal = final_.ordinal()?;
        let occurrence = needed(&facts.occurrences, ordinal, None)?;
        let result = needed(
            &facts.results,
            ordinal,
            Some(final_.number("terminal_attempt_ordinal")?),
        )?;
        require(
            final_.text("code")? == occurrence.text("code")?
                && final_.text("provenance")? == "PositionConceptRpc"
                && final_.number("occurrence_run_version")? == occurrence.version()?
                && final_.text("occurrence_request_sha256")?
                    == occurrence.text("request_sha256")?
                && final_.number("terminal_result_run_version")? == result.version()?
                && final_.text("terminal_result_sha256")? == result.text("result_sha256")?
                && result.text("continuation")? == "Terminal",
        )?;
        final_.parent(result, "applied_at", "committed_at", false)?;
        let error = find(&facts.errors, ordinal, None)?;
        let projected = if final_.text("final_outcome")? == "Error" {
            let error = error.ok_or(Error::SchemaRejected)?;
            require(
                final_.number("error_material_run_version")? == error.version()?
                    && final_.text("error_material_sha256")? == error.text("material_sha256")?,
            )?;
            final_.parent(error, "applied_at", "captured_at", false)?;
            let (gateway, _) = board_error_codec::decode_error(error.blob("material")?)?;
            Err(restore_gateway_error(&gateway).map_err(|_| Error::SchemaRejected)?)
        } else {
            require(
                error.is_none()
                    && final_.null("error_material_run_version")?
                    && final_.null("error_material_sha256")?,
            )?;
            let request = request(occurrence)?;
            ConnectedBoardQueries::restore_memberships_response(
                request.profile,
                request.acquisition_authority.as_deref(),
                &request.request_id,
                codec::decode_result(result.blob("result")?)?
                    .terminal_response()?
                    .ok_or(Error::SchemaRejected)?,
            )
        };
        let error_audit = error
            .map(|row| {
                board_error_codec::decode_error(row.blob("material")?).map(|(_, audit)| audit)
            })
            .transpose()?;
        let (outcome, raw, audit) = super::concept_rpc::project_membership(
            occurrence.text("code")?,
            &projected,
            error_audit.as_ref(),
            &timestamp(
                UtcMicros::try_new(final_.integer("applied_at")?)
                    .map_err(|_| Error::SchemaRejected)?,
            )?,
        )?;
        require(
            codec::decode_final(final_.blob("final")?)? == (outcome.clone(), raw.clone())
                && final_.text("final_outcome")? == outcome,
        )?;
        if outcome == "Available" {
            require(
                !super::parse_concept_provider_raw(&raw, occurrence.text("code")?)
                    .map_err(|_| Error::SchemaRejected)?
                    .is_empty(),
            )?;
        }
        let receipt = receipt(final_)?;
        verify_acquisition_receipt_in_transaction(
            transaction,
            &receipt,
            &audit.borrowed("board-memberships"),
        )
        .map_err(|_| Error::SchemaRejected)?;
        let reused: i64 = transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_concept_rpc_finals WHERE audit_id=?1",
                [receipt.audit_id],
                |row| row.get(0),
            )
            .map_err(|_| storage("position RPC receipt ownership"))?;
        require(reused == 0)?;
    }
    let mut applied = HashSet::new();
    for cache in &facts.caches {
        validate_cache_history_barrier(facts, &batch, cache)?;
        let ordinal = cache.ordinal()?;
        let final_ = needed(&facts.finals, ordinal, None)?;
        require(
            applied.insert(ordinal)
                && cache.text("code")? == final_.text("code")?
                && final_.text("final_outcome")? == "Available"
                && cache.number("final_run_version")? == final_.version()?
                && cache.text("final_sha256")? == final_.text("final_sha256")?
                && cache.number("terminal_result_run_version")?
                    == final_.number("terminal_result_run_version")?,
        )?;
        let (_, raw) = codec::decode_final(final_.blob("final")?)?;
        let parsed = super::parse_concept_provider_raw(&raw, cache.text("code")?)
            .map_err(|_| Error::SchemaRejected)?;
        let bytes = cache.blob("concepts")?;
        require(serde_json::to_vec(&parsed).map_err(|_| Error::SchemaRejected)? == bytes)?;
        let updated = cache.text("cache_updated_at")?;
        let date = chrono::NaiveDateTime::parse_from_str(updated, "%Y-%m-%d %H:%M:%S")
            .map_err(|_| Error::SchemaRejected)?;
        require(date.format("%Y-%m-%d %H:%M:%S").to_string() == updated)?;
        for earlier in &facts.finals {
            if earlier.number("terminal_result_run_version")?
                < cache.number("terminal_result_run_version")?
            {
                require(
                    earlier.text("final_outcome")? == "Available"
                        && applied.contains(&earlier.ordinal()?),
                )?;
                let previous = needed(&facts.caches, earlier.ordinal()?, None)?;
                require(
                    previous.version()? < cache.version()?
                        && previous.integer("written_at")? <= cache.integer("written_at")?,
                )?;
            }
        }
    }
    Ok(())
}
fn receipt(fact: &Fact) -> Result<DataAcquisitionAuditReceipt> {
    Ok(DataAcquisitionAuditReceipt {
        audit_id: fact.integer("audit_id")?,
        record_hash: fact.text("audit_record_hash")?.to_owned(),
        previous_outcome: fact.optional_text("previous_outcome")?.map(str::to_owned),
        current_outcome: fact.text("current_outcome")?.to_owned(),
    })
}
pub(super) fn validate_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
) -> Result<()> {
    validate_facts_scoped(transaction, intent, recovery, layout_version, None)
}

pub(super) fn validate_facts_scoped(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<()> {
    validate_facts_with_position_validation(
        transaction,
        intent,
        recovery,
        layout_version,
        proof,
        None,
    )
}

/// Only the current v12 read pass may borrow its already-validated parent;
/// neither the token nor its projection may cross a write or another pass.
pub(super) fn validate_facts_after_position_validation(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    proof: &schema::V12CatalogProof<'_, '_>,
    positions: &positions::ValidatedPositionFacts<'_, '_, '_>,
) -> Result<()> {
    validate_facts_with_position_validation(
        transaction,
        intent,
        recovery,
        proof.layout(),
        Some(proof),
        Some(positions),
    )
}

fn validate_facts_with_position_validation(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
    positions: Option<&positions::ValidatedPositionFacts<'_, '_, '_>>,
) -> Result<()> {
    super::validate_owned_foreign_keys(transaction)?;
    validate_acquisition_chain_in_transaction(transaction).map_err(|_| Error::SchemaRejected)?;
    validate_rows(
        transaction,
        intent,
        recovery,
        &Facts::read(transaction, intent)?,
        layout_version,
        proof,
        positions,
    )
}

pub(super) fn completed_parent_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    batch: &PositionBatch,
) -> Result<Vec<positions::PositionRpcCompletion>> {
    let facts = Facts::read(transaction, intent)?;
    validate_rows_with_batch(transaction, intent, recovery, &facts, 10, batch, None)?;
    require(!batch.work.is_empty())?;
    require(
        facts.occurrences.len() == batch.work.len()
            && facts.finals.len() == batch.work.len()
            && facts.caches.len() == batch.work.len(),
    )?;
    let mut completion = Vec::with_capacity(batch.work.len());
    for (ordinal, code) in &batch.work {
        let occurrence = needed(&facts.occurrences, *ordinal, None)?;
        let final_ = needed(&facts.finals, *ordinal, None)?;
        let cache = needed(&facts.caches, *ordinal, None)?;
        let terminal = needed(
            &facts.results,
            *ordinal,
            Some(final_.number("terminal_attempt_ordinal")?),
        )?;
        require(
            occurrence.text("code")? == code
                && final_.text("final_outcome")? == "Available"
                && cache.text("code")? == code
                && cache.number("final_run_version")? == final_.version()?
                && cache.text("final_sha256")? == final_.text("final_sha256")?,
        )?;
        completion.push(positions::PositionRpcCompletion {
            ordinal: *ordinal,
            code: code.clone(),
            occurrence_version: occurrence.version()?,
            occurrence_digest: occurrence.text("request_sha256")?.to_owned(),
            occurrence_owner: occurrence.text("lease_owner")?.to_owned(),
            occurrence_generation: occurrence.number("lease_generation")?,
            occurrence_time: occurrence.integer("planned_at")?,
            terminal_version: terminal.version()?,
            terminal_digest: terminal.text("result_sha256")?.to_owned(),
            terminal_owner: terminal.text("lease_owner")?.to_owned(),
            terminal_generation: terminal.number("lease_generation")?,
            terminal_time: terminal.integer("committed_at")?,
            final_version: final_.version()?,
            final_digest: final_.text("final_sha256")?.to_owned(),
            final_owner: final_.text("lease_owner")?.to_owned(),
            final_generation: final_.number("lease_generation")?,
            final_time: final_.integer("applied_at")?,
            cache_version: cache.version()?,
            cache_digest: cache.text("concepts_sha256")?.to_owned(),
            cache_owner: cache.text("lease_owner")?.to_owned(),
            cache_generation: cache.number("lease_generation")?,
            cache_time: cache.integer("written_at")?,
        });
    }
    Ok(completion)
}
fn admit(
    transaction: &Transaction<'_>,
    lease: &RunLease,
    batch: &PositionBatch,
    now: UtcMicros,
) -> Result<Facts> {
    let layout_version = schema::runtime_layout_version(transaction)?;
    if layout_version >= 12 {
        schema::verify_parent_layout_v12(transaction)?;
    } else if !matches!(layout_version, 9 | 10 | 11) {
        return Err(Error::UnsupportedVersion);
    }
    let recovery = inspect_run_on(transaction, &lease.intent_id)?;
    check_lease(transaction, lease, now)?;
    if recovery.context.run_id() != &lease.run_id
        || recovery.input.encode()? != lease.input.encode()?
        || recovery.generation != lease.generation
        || recovery.head != lease.head
    {
        return Err(Error::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    let loaded = positions::load_position_batch_parent_at_layout(
        transaction,
        &lease.intent_id,
        &recovery,
        layout_version,
    )?
    .ok_or(Error::SchemaRejected)?
    .into_batch()
    .map_err(|_| Error::SchemaRejected)?;
    require(
        loaded.parent.intent_id == batch.parent.intent_id
            && loaded.parent.run_id == batch.parent.run_id
            && loaded.parent.cache_version == batch.parent.cache_version
            && loaded.parent.cache_digest == batch.parent.cache_digest
            && loaded.parent.positions_version == batch.parent.positions_version
            && loaded.parent.positions_digest == batch.parent.positions_digest
            && loaded.parent.context_digest == batch.parent.context_digest
            && loaded.parent.input_digest == batch.parent.input_digest
            && loaded.parent.requested_codes == batch.parent.requested_codes
            && loaded.work == batch.work
            && loaded.cached == batch.cached,
    )?;
    Facts::read(transaction, &lease.intent_id)
}
fn advance(transaction: &Transaction<'_>, lease: &mut RunLease, now: UtcMicros) -> Result<u64> {
    let previous = lease.head;
    let next = previous
        .checked_add(1)
        .ok_or_else(|| storage("position RPC head"))?;
    let changed = transaction.execute(
        "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 WHERE intent_id=?3 AND run_id=?4 AND lease_owner=?5 AND lease_generation=?6 AND head_version=?7 AND lease_until>?2",
        params![next, now.get(), lease.intent_id.as_str(), lease.run_id.as_str(), lease.owner.as_str(), lease.generation, previous],
    ).map_err(|_| storage("position RPC cas"))?;
    if changed != 1 {
        return Err(Error::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    lease.head = next;
    Ok(previous)
}
fn validate_written(transaction: &Transaction<'_>, lease: &RunLease) -> Result<()> {
    let layout_version = schema::runtime_layout_version(transaction)?;
    if layout_version >= 12 {
        schema::verify_parent_layout_v12(transaction)?;
    } else if !matches!(layout_version, 9 | 10 | 11) {
        return Err(Error::UnsupportedVersion);
    }
    super::validate_run_fact_versions_at_layout(
        transaction,
        &lease.intent_id,
        lease.head,
        layout_version,
    )?;
    let recovery = super::inspect_run_on_at_layout(transaction, &lease.intent_id, layout_version)?;
    validate_facts(transaction, &lease.intent_id, &recovery, layout_version)
}

impl LocalChainPostClose<'_> {
    pub(super) fn load_position_rpc(
        &mut self,
        lease: &RunLease,
        batch: &PositionBatch,
        ordinal: u64,
        code: &str,
        now: UtcMicros,
    ) -> Result<Recovery> {
        let scope = Scope::from_batch(batch, ordinal, code)?;
        scope.check(batch, lease)?;
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let facts = admit(&transaction, lease, batch, now)?;
        let recovery = if let Some(final_) = find(&facts.finals, ordinal, None)? {
            Recovery::Complete(projection(scope, final_)?)
        } else if let Some(occurrence) = find(&facts.occurrences, ordinal, None)? {
            let last = facts
                .begins
                .iter()
                .filter(|row| row.ordinal().ok() == Some(ordinal))
                .last();
            match last {
                None => Recovery::Planned {
                    request: restored(request(occurrence)?, 1),
                    occurrence_version: occurrence.version()?,
                },
                Some(begin) => {
                    let attempt = begin.number("attempt_ordinal")?;
                    match find(&facts.results, ordinal, Some(attempt))? {
                        None => Recovery::BegunUnconfirmed,
                        Some(result) => {
                            let terminal = terminal(scope, result, &facts)?;
                            let decoded = codec::decode_result(&terminal.bytes)?;
                            if let Some(backoff_ms) = decoded.confirmed_retry_backoff()? {
                                Recovery::Retry {
                                    request: restored(
                                        request(occurrence)?,
                                        u32::try_from(attempt + 1)
                                            .map_err(|_| Error::SchemaRejected)?,
                                    ),
                                    occurrence_version: occurrence.version()?,
                                    backoff_ms,
                                }
                            } else if let Some(error) = find(&facts.errors, ordinal, None)? {
                                let material = material(terminal.scope.clone(), error)?;
                                Recovery::Error { terminal, material }
                            } else if let Some(response) = decoded.terminal_response()? {
                                let request = request(occurrence)?;
                                if ConnectedBoardQueries::restore_memberships_response(
                                    request.profile,
                                    request.acquisition_authority.as_deref(),
                                    &request.request_id,
                                    response.clone(),
                                )
                                .is_err()
                                {
                                    Recovery::TerminalUnconfirmed
                                } else {
                                    Recovery::Response {
                                        terminal,
                                        request,
                                        response,
                                    }
                                }
                            } else {
                                Recovery::TerminalUnconfirmed
                            }
                        }
                    }
                }
            }
        } else {
            Recovery::NeverStarted
        };
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        Ok(recovery)
    }

    pub(super) fn begin_position_rpc(
        &mut self,
        mut lease: RunLease,
        batch: &PositionBatch,
        ordinal: u64,
        code: &str,
        occurrence_version: Option<u64>,
        authorized: &AuthorizedBoardAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, Call)> {
        let scope = Scope::from_batch(batch, ordinal, code)?;
        scope.check(batch, &lease)?;
        let bytes = codec::request_bytes(
            code,
            authorized.request_id(),
            authorized.request_bytes(),
            authorized.profile(),
            authorized.acquisition_authority(),
            authorized.retry_policy(),
        )?;
        codec::decode_request(&bytes)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let facts = admit(&transaction, &lease, batch, now)?;
        let parent = &batch.parent;
        let occurrence = match occurrence_version {
            Some(version) => {
                let stored = needed(&facts.occurrences, ordinal, None)?;
                require(stored.version()? == version && stored.blob("request")? == bytes)?;
                version
            }
            None => {
                require(find(&facts.occurrences, ordinal, None)?.is_none())?;
                let previous = advance(&transaction, &mut lease, now)?;
                transaction.execute(
                    "INSERT INTO chain_post_close_position_concept_rpc_occurrences(
                     intent_id,cache_material_run_version,position_ordinal,code,cache_material_sha256,positions_run_version,positions_sha256,
                     operation,request_id,request_codec_version,request_bytes,request_length,request_sha256,acquisition_request_hash,profile,acquisition_authority,
                     retry_max_attempts,retry_base_delay_ms,retry_max_delay_ms,retry_jitter_ms,
                     run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,planned_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,'BoardConstituents',?8,1,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)",
                    params![lease.intent_id.as_str(), parent.cache_version, ordinal, code, parent.cache_digest, parent.positions_version, parent.positions_digest,
                        authorized.request_id(), &bytes, bytes.len() as u64, &digest, crate::data_gateway::review::board_membership_request_hash(code), authorized.profile(), authorized.acquisition_authority(),
                        authorized.retry_policy().0, authorized.retry_policy().1, authorized.retry_policy().2, authorized.retry_policy().3,
                        parent.run_id, parent.context_digest, parent.input_digest, lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
                ).map_err(|_| storage("position RPC occurrence fact"))?;
                lease.head
            }
        };
        let attempt = authorized.attempt_ordinal();
        let previous_result = if attempt > 1 {
            Some(needed(
                &facts.results,
                ordinal,
                Some(u64::from(attempt - 1)),
            )?)
        } else {
            None
        };
        let previous = advance(&transaction, &mut lease, now)?;
        transaction.execute(
            "INSERT INTO chain_post_close_position_concept_rpc_attempt_begins(
             intent_id,cache_material_run_version,position_ordinal,attempt_ordinal,occurrence_run_version,request_id,request_sha256,
             previous_attempt_ordinal,previous_result_run_version,previous_result_sha256,
             run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,begun_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![lease.intent_id.as_str(), parent.cache_version, ordinal, attempt, occurrence, authorized.request_id(), &digest,
                previous_result.map(|_| attempt - 1), previous_result.map(Fact::version).transpose()?, previous_result.map(|row| row.text("result_sha256")).transpose()?,
                parent.run_id, parent.context_digest, parent.input_digest, lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
        ).map_err(|_| storage("position RPC begin fact"))?;
        validate_written(&transaction, &lease)?;
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        let call = Call {
            scope,
            occurrence,
            attempt,
            begin: lease.head,
            request_digest: digest,
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
        };
        Ok((lease, call))
    }

    pub(super) fn record_position_rpc(
        &mut self,
        mut lease: RunLease,
        batch: &PositionBatch,
        call: Call,
        completion: &BoardAttemptCompletion,
        now: UtcMicros,
    ) -> Result<(RunLease, Terminal, Option<LiveErrorCapability>)> {
        call.scope.check(batch, &lease)?;
        require(call.owner == lease.owner.as_str() && call.generation == lease.generation)?;
        let bytes = codec::result_bytes(completion)?;
        let decoded = codec::decode_result(&bytes)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let facts = admit(&transaction, &lease, batch, now)?;
        let begin = needed(
            &facts.begins,
            call.scope.ordinal,
            Some(u64::from(call.attempt)),
        )?;
        require(
            begin.version()? == call.begin && begin.text("request_sha256")? == call.request_digest,
        )?;
        let parent = &batch.parent;
        let previous = advance(&transaction, &mut lease, now)?;
        let result_version = lease.head;
        transaction.execute(
            "INSERT INTO chain_post_close_position_concept_rpc_attempt_results(
             intent_id,cache_material_run_version,position_ordinal,attempt_ordinal,begin_run_version,request_sha256,
             wire_outcome,result_codec_version,result_bytes,result_length,result_sha256,continuation,retry_decision,backoff_ms,
             run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,returned_at,committed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?21)",
            params![lease.intent_id.as_str(), parent.cache_version, call.scope.ordinal, call.attempt, call.begin, &call.request_digest,
                if completion.response_bytes.is_some() { "Response" } else { "Status" }, &bytes, bytes.len() as u64, &digest,
                decoded.continuation(), decoded.retry_decision(), decoded.backoff_ms(),
                parent.run_id, parent.context_digest, parent.input_digest, lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
        ).map_err(|_| storage("position RPC result fact"))?;
        let status = if completion.response_bytes.is_none() {
            let diagnostic = completion
                .processed
                .as_ref()
                .err()
                .ok_or(Error::SchemaRejected)?
                .safe_diagnostic();
            let material_bytes = board_error_codec::status_bytes(diagnostic)?;
            let material_digest = raw_digest(&material_bytes).as_str().to_owned();
            let previous = advance(&transaction, &mut lease, now)?;
            transaction.execute(
                "INSERT INTO chain_post_close_position_concept_rpc_status_materials(
                 intent_id,cache_material_run_version,position_ordinal,attempt_ordinal,result_run_version,result_sha256,request_sha256,
                 provenance,projection_version,material_codec_version,material_bytes,material_length,material_sha256,
                 run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,captured_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,'Captured',1,1,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                params![lease.intent_id.as_str(), parent.cache_version, call.scope.ordinal, call.attempt, result_version, &digest, &call.request_digest,
                    &material_bytes, material_bytes.len() as u64, &material_digest, parent.run_id, parent.context_digest, parent.input_digest,
                    lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
            ).map_err(|_| storage("position RPC status fact"))?;
            Some((lease.head, material_digest))
        } else {
            None
        };
        validate_written(&transaction, &lease)?;
        // The capability does not exist until COMMIT returns successfully.
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        let terminal = Terminal {
            scope: call.scope,
            attempt: call.attempt,
            version: result_version,
            digest,
            request_digest: call.request_digest,
            bytes,
            status,
        };
        let capability = (decoded.continuation() == "Terminal").then(|| LiveErrorCapability {
            terminal: terminal.clone(),
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
            head: lease.head,
        });
        Ok((lease, terminal, capability))
    }

    pub(super) fn confirm_position_rpc_error(
        &mut self,
        mut lease: RunLease,
        batch: &PositionBatch,
        capability: LiveErrorCapability,
        error: &GatewayError,
        now: UtcMicros,
    ) -> Result<(RunLease, Material)> {
        capability.terminal.scope.check(batch, &lease)?;
        require(
            capability.owner == lease.owner.as_str()
                && capability.generation == lease.generation
                && capability.head == lease.head,
        )?;
        let terminal = capability.terminal;
        let observed = timestamp(now)?;
        let projected: Projected = Err(restore_gateway_error(&store_gateway_error(error))
            .map_err(|_| Error::SchemaRejected)?);
        let audit = map_gateway_audit_record(
            "board-memberships",
            ProviderId::Tdx,
            &crate::data_gateway::review::board_membership_request_hash(&terminal.scope.code),
            &projected,
            &observed,
        )
        .map_err(|_| Error::SchemaRejected)?;
        let bytes = board_error_codec::error_bytes(store_gateway_error(error), audit.clone())?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        admit(&transaction, &lease, batch, now)?;
        let parent = &batch.parent;
        let previous = advance(&transaction, &mut lease, now)?;
        transaction.execute(
            "INSERT INTO chain_post_close_position_concept_rpc_error_materials(
             intent_id,cache_material_run_version,position_ordinal,terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256,request_sha256,
             status_material_attempt_ordinal,status_material_run_version,status_material_sha256,
             material_codec_version,material_bytes,material_length,material_sha256,observed_fallback,
             run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,captured_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            params![lease.intent_id.as_str(), parent.cache_version, terminal.scope.ordinal, terminal.attempt, terminal.version, terminal.digest, terminal.request_digest,
                terminal.status.as_ref().map(|_| terminal.attempt), terminal.status.as_ref().map(|value| value.0), terminal.status.as_ref().map(|value| value.1.as_str()),
                &bytes, bytes.len() as u64, &digest, &observed, parent.run_id, parent.context_digest, parent.input_digest, lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
        ).map_err(|_| storage("position RPC error fact"))?;
        validate_written(&transaction, &lease)?;
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        let material = Material {
            scope: terminal.scope,
            version: lease.head,
            digest,
            gateway: projected.unwrap_err(),
            audit,
        };
        Ok((lease, material))
    }

    pub(super) fn finalize_position_rpc(
        &mut self,
        mut lease: RunLease,
        batch: &PositionBatch,
        terminal: &Terminal,
        projected: &Projected,
        material: Option<&Material>,
        now: UtcMicros,
    ) -> Result<(RunLease, Projection)> {
        terminal.scope.check(batch, &lease)?;
        require(material.is_none_or(|value| value.scope == terminal.scope))?;
        let (outcome, raw, audit) = super::concept_rpc::project_membership(
            &terminal.scope.code,
            projected,
            material.map(|value| &value.audit),
            &timestamp(now)?,
        )?;
        let bytes = codec::final_bytes(&outcome, raw.clone())?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let facts = admit(&transaction, &lease, batch, now)?;
        let occurrence = needed(&facts.occurrences, terminal.scope.ordinal, None)?;
        let persisted = needed(
            &facts.results,
            terminal.scope.ordinal,
            Some(u64::from(terminal.attempt)),
        )?;
        require(
            persisted.version()? == terminal.version
                && persisted.blob("result")? == terminal.bytes
                && persisted.text("result_sha256")? == terminal.digest
                && occurrence.text("request_sha256")? == terminal.request_digest
                && find(&facts.finals, terminal.scope.ordinal, None)?.is_none(),
        )?;
        let borrowed = audit.borrowed("board-memberships");
        let receipt = append_acquisition_in_transaction(&transaction, &borrowed)
            .map_err(|_| storage("position RPC audit append"))?;
        let parent = &batch.parent;
        let previous = advance(&transaction, &mut lease, now)?;
        transaction.execute(
            "INSERT INTO chain_post_close_position_concept_rpc_finals(
             intent_id,cache_material_run_version,position_ordinal,code,provenance,occurrence_run_version,occurrence_request_sha256,
             terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256,error_material_run_version,error_material_sha256,
             final_outcome,final_codec_version,final_bytes,final_length,final_sha256,audit_id,audit_record_hash,previous_outcome,current_outcome,
             run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,applied_at)
             VALUES(?1,?2,?3,?4,'PositionConceptRpc',?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)",
            params![lease.intent_id.as_str(), parent.cache_version, terminal.scope.ordinal, terminal.scope.code, occurrence.version()?, terminal.request_digest,
                terminal.attempt, terminal.version, terminal.digest, material.map(|value| value.version), material.map(|value| value.digest.as_str()),
                &outcome, &bytes, bytes.len() as u64, &digest, receipt.audit_id, receipt.record_hash, receipt.previous_outcome, receipt.current_outcome,
                parent.run_id, parent.context_digest, parent.input_digest, lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
        ).map_err(|_| storage("position RPC final fact"))?;
        verify_acquisition_receipt_in_transaction(&transaction, &receipt, &borrowed)
            .map_err(|_| Error::SchemaRejected)?;
        validate_written(&transaction, &lease)?;
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        let projection = Projection {
            scope: terminal.scope.clone(),
            final_version: lease.head,
            final_digest: digest,
            terminal_version: terminal.version,
            outcome,
            raw,
        };
        Ok((lease, projection))
    }

    pub(super) fn apply_position_rpc_cache(
        &mut self,
        mut lease: RunLease,
        batch: &PositionBatch,
        projection: &Projection,
        now: UtcMicros,
    ) -> Result<(RunLease, Vec<String>)> {
        projection.scope.check(batch, &lease)?;
        require(projection.outcome == "Available")?;
        let concepts = super::parse_concept_provider_raw(&projection.raw, &projection.scope.code)
            .map_err(|_| Error::SchemaRejected)?;
        let bytes = serde_json::to_vec(&concepts).map_err(|_| Error::SchemaRejected)?;
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let facts = admit(&transaction, &lease, batch, now)?;
        // The ordinary reader permits partial prefixes; cache admission does not.
        require(
            facts.occurrences.len() == batch.work.len() && facts.finals.len() == batch.work.len(),
        )?;
        let final_ = needed(&facts.finals, projection.scope.ordinal, None)?;
        require(
            final_.version()? == projection.final_version
                && final_.text("final_sha256")? == projection.final_digest
                && final_.number("terminal_result_run_version")? == projection.terminal_version,
        )?;
        if let Some(saved) = find(&facts.caches, projection.scope.ordinal, None)? {
            let parsed = serde_json::from_slice(saved.blob("concepts")?)
                .map_err(|_| Error::SchemaRejected)?;
            if transaction.commit().is_err() {
                if !self.store.connection.is_autocommit() {
                    let _ = self.store.connection.execute_batch("ROLLBACK;");
                }
                return Err(storage("commit"));
            }
            return Ok((lease, parsed));
        }
        for earlier in &facts.finals {
            if earlier.number("terminal_result_run_version")? < projection.terminal_version {
                require(
                    earlier.text("final_outcome")? == "Available"
                        && find(&facts.caches, earlier.ordinal()?, None)?.is_some(),
                )?;
            }
        }
        let updated = Utc
            .timestamp_micros(now.get())
            .single()
            .ok_or(Error::SchemaRejected)?
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let parent = &batch.parent;
        let previous = advance(&transaction, &mut lease, now)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO stock_concepts(code,concepts,updated_at) VALUES(?1,?2,?3)",
                params![
                    projection.scope.code,
                    std::str::from_utf8(&bytes).map_err(|_| Error::SchemaRejected)?,
                    &updated
                ],
            )
            .map_err(|_| storage("position RPC cache write"))?;
        transaction.execute(
            "INSERT INTO chain_post_close_position_concept_cache_writes(
             intent_id,cache_material_run_version,position_ordinal,code,final_run_version,final_sha256,terminal_result_run_version,
             concepts_codec_version,concepts_bytes,concepts_length,concepts_sha256,cache_updated_at,
             run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,written_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            params![lease.intent_id.as_str(), parent.cache_version, projection.scope.ordinal, projection.scope.code,
                projection.final_version, projection.final_digest, projection.terminal_version, &bytes, bytes.len() as u64, raw_digest(&bytes).as_str(), updated,
                parent.run_id, parent.context_digest, parent.input_digest, lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
        ).map_err(|_| storage("position RPC cache fact"))?;
        validate_written(&transaction, &lease)?;
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        Ok((lease, concepts))
    }
}
