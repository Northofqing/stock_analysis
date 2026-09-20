use chrono::{SecondsFormat, TimeZone as _, Utc};
use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::data_gateway::grpc_source::{
    BoardAttemptCompletion, BoardContinuation, ConnectedBoardQueries, RestoredMembershipRequest,
};
use crate::data_gateway::review::{
    map_gateway_audit_record, restore_gateway_error, store_gateway_error, OwnedGatewayAuditRecord,
};
use crate::data_gateway::{GatewayBatch, GatewayError};
use crate::database::data_acquisition_audit::{
    append_acquisition_in_transaction, validate_acquisition_chain_in_transaction,
    verify_acquisition_receipt_in_transaction, DataAcquisitionAuditReceipt,
};
use crate::market_domain::ProviderId;
use crate::monitor::push_job::{raw_digest, IntentId, RunId, UtcMicros};

use super::concept_rpc_codec;
use super::{
    board_error_codec, check_lease, inspect_run_on, schema, storage, validate_run_fact_versions,
    ChainPostCloseError, LocalChainPostClose, RunLease, StoredConceptProviderResult,
};

fn verify_concept_rpc_layout(connection: &Connection) -> Result<(), ChainPostCloseError> {
    match schema::runtime_layout_version(connection)? {
        // The current runtime reader attests the exact catalog before returning.
        7 | 8 | 9 | 10 | 11 | 12 | 13 => Ok(()),
        _ => Err(ChainPostCloseError::UnsupportedVersion),
    }
}

pub(super) struct ConceptRpcCall {
    intent_id: IntentId,
    run_id: RunId,
    ordinal: u64,
    attempt: u32,
    occurrence_version: u64,
    begin_version: u64,
    request_digest: String,
    owner: String,
    generation: u64,
}

impl ConceptRpcCall {
    pub(super) fn occurrence_version(&self) -> u64 {
        self.occurrence_version
    }
}

pub(super) struct StoredAttemptResult {
    ordinal: u64,
    attempt: u32,
    run_version: u64,
    digest: String,
    request_digest: String,
    bytes: Vec<u8>,
    status_material: Option<(u64, String)>,
}

impl StoredAttemptResult {
    pub(super) fn retry_backoff(&self) -> Result<Option<u64>, ChainPostCloseError> {
        concept_rpc_codec::decode_result(&self.bytes)?.confirmed_retry_backoff()
    }
}

pub(super) struct LiveErrorCapability {
    intent_id: IntentId,
    run_id: RunId,
    terminal: StoredAttemptResult,
    owner: String,
    generation: u64,
    head: u64,
}

pub(super) struct StoredErrorMaterial {
    run_version: u64,
    digest: String,
    gateway: GatewayError,
    audit: OwnedGatewayAuditRecord,
}

impl StoredErrorMaterial {
    pub(super) fn gateway_error(&self) -> GatewayError {
        restore_gateway_error(&store_gateway_error(&self.gateway))
            .expect("validated stored gateway error remains restorable")
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
        terminal: StoredAttemptResult,
        request: concept_rpc_codec::RestoredRequest,
        response: crate::grpc_client::pb::magic::market::v1::QueryResponse,
    },
    Error {
        terminal: StoredAttemptResult,
        material: StoredErrorMaterial,
    },
    TerminalUnconfirmed,
    Complete(StoredConceptProviderResult),
}

impl LocalChainPostClose<'_> {
    pub(super) fn load_concept_rpc(
        &mut self,
        lease: &RunLease,
        ordinal: u64,
        code: &str,
        now: UtcMicros,
    ) -> Result<Recovery, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        verify_concept_rpc_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_lease_identity(&recovery, lease)?;
        check_lease(&transaction, lease, now)?;
        validate_run_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if let Some(result) = load_final(&transaction, &lease.intent_id, ordinal, code)? {
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(Recovery::Complete(result));
        }
        let occurrence = load_occurrence(&transaction, &lease.intent_id, ordinal, code)?;
        let Some(occurrence) = occurrence else {
            let legacy = load_legacy_recovery(&transaction, &lease.intent_id, ordinal, code)?;
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(legacy.unwrap_or(Recovery::NeverStarted));
        };
        let request = concept_rpc_codec::decode_request(&occurrence.request_bytes)?;
        if request.code != code {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let last = load_last_attempt(&transaction, &lease.intent_id, ordinal)?;
        let state = match last {
            None => Recovery::Planned {
                request: restored_request(request, 1),
                occurrence_version: occurrence.run_version,
            },
            Some((_attempt, _, None)) => Recovery::BegunUnconfirmed,
            Some((attempt, _, Some(result))) => {
                let decoded = concept_rpc_codec::decode_result(&result.bytes)?;
                if let Some(backoff_ms) = decoded.confirmed_retry_backoff()? {
                    Recovery::Retry {
                        request: restored_request(request, attempt.saturating_add(1)),
                        occurrence_version: occurrence.run_version,
                        backoff_ms,
                    }
                } else if let Some(material) =
                    load_error_material(&transaction, &lease.intent_id, ordinal)?
                {
                    Recovery::Error {
                        terminal: result,
                        material,
                    }
                } else if let Some(response) = decoded.terminal_response()? {
                    Recovery::Response {
                        terminal: result,
                        request,
                        response,
                    }
                } else if decoded.terminal_status()?.is_some() {
                    Recovery::TerminalUnconfirmed
                } else {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
            }
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(state)
    }

    pub(super) fn begin_concept_rpc_attempt(
        &mut self,
        mut lease: RunLease,
        ordinal: u64,
        code: &str,
        occurrence_version: Option<u64>,
        authorized: &crate::grpc_client::client::board_attempt::AuthorizedBoardAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, ConceptRpcCall), ChainPostCloseError> {
        let request_bytes = concept_rpc_codec::request_bytes(
            code,
            authorized.request_id(),
            authorized.request_bytes(),
            authorized.profile(),
            authorized.acquisition_authority(),
            authorized.retry_policy(),
        )?;
        let request_digest = raw_digest(&request_bytes).as_str().to_owned();
        let request_hash = crate::data_gateway::review::board_membership_request_hash(code);
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_concept_rpc_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_lease_identity(&recovery, &lease)?;
        check_lease(&transaction, &lease, now)?;
        validate_run_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        let occurrence_version = match occurrence_version {
            Some(version) => {
                let existing = load_occurrence(&transaction, &lease.intent_id, ordinal, code)?
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                if existing.run_version != version
                    || existing.request_bytes != request_bytes
                    || existing.request_digest != request_digest
                {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                version
            }
            None => {
                if load_occurrence(&transaction, &lease.intent_id, ordinal, code)?.is_some() {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                let previous = lease.head;
                advance_run(&transaction, &mut lease, now, "concept RPC occurrence cas")?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_concept_rpc_occurrences( \
                         intent_id,outer_ordinal,code,operation,request_id,request_codec_version, \
                         request_bytes,request_length,request_sha256,acquisition_request_hash, \
                         profile,acquisition_authority,retry_max_attempts,retry_base_delay_ms, \
                         retry_max_delay_ms,retry_jitter_ms,run_id,run_context_sha256,input_sha256, \
                         lease_owner,lease_generation,prior_head_version,run_version,planned_at) \
                         VALUES(?1,?2,?3,'BoardConstituents',?4,1,?5,?6,?7,?8,?9,?10, \
                                ?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
                        params![
                            lease.intent_id.as_str(), ordinal, code, authorized.request_id(),
                            &request_bytes,
                            i64::try_from(request_bytes.len())
                                .map_err(|_| storage("concept RPC request length"))?,
                            &request_digest, &request_hash, authorized.profile(),
                            authorized.acquisition_authority(), authorized.retry_policy().0,
                            authorized.retry_policy().1, authorized.retry_policy().2,
                            authorized.retry_policy().3, recovery.context.run_id().as_str(),
                            recovery.context.canonical_sha256().as_str(),
                            raw_digest(&recovery.input.encode()?).as_str(), lease.owner.as_str(),
                            lease.generation, previous, lease.head, now.get()
                        ],
                    )
                    .map_err(|_| storage("concept RPC occurrence fact"))?;
                lease.head
            }
        };
        let attempt = authorized.attempt_ordinal();
        let previous_result = if attempt == 1 {
            None
        } else {
            load_attempt_result(&transaction, &lease.intent_id, ordinal, attempt - 1)?
        };
        let previous = lease.head;
        advance_run(&transaction, &mut lease, now, "concept RPC begin cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_concept_rpc_attempt_begins( \
                 intent_id,outer_ordinal,attempt_ordinal,occurrence_run_version,request_id, \
                 request_sha256,previous_attempt_ordinal,previous_result_run_version, \
                 previous_result_sha256,run_id,run_context_sha256,input_sha256,lease_owner, \
                 lease_generation,prior_head_version,run_version,begun_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![
                    lease.intent_id.as_str(),
                    ordinal,
                    attempt,
                    occurrence_version,
                    authorized.request_id(),
                    &request_digest,
                    previous_result.as_ref().map(|value| value.attempt),
                    previous_result.as_ref().map(|value| value.run_version),
                    previous_result.as_ref().map(|value| value.digest.as_str()),
                    recovery.context.run_id().as_str(),
                    recovery.context.canonical_sha256().as_str(),
                    raw_digest(&recovery.input.encode()?).as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get()
                ],
            )
            .map_err(|_| storage("concept RPC begin fact"))?;
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        validate_concept_rpc_fact_rows(&transaction, &lease.intent_id)?;
        let call = ConceptRpcCall {
            intent_id: lease.intent_id.clone(),
            run_id: lease.run_id.clone(),
            ordinal,
            attempt,
            occurrence_version,
            begin_version: lease.head,
            request_digest,
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, call))
    }

    pub(super) fn record_concept_rpc_result(
        &mut self,
        mut lease: RunLease,
        call: ConceptRpcCall,
        completion: &BoardAttemptCompletion,
        now: UtcMicros,
    ) -> Result<(RunLease, StoredAttemptResult, Option<LiveErrorCapability>), ChainPostCloseError>
    {
        if call.intent_id != lease.intent_id
            || call.run_id != lease.run_id
            || call.owner != lease.owner.as_str()
            || call.generation != lease.generation
        {
            return Err(stale(&lease));
        }
        let bytes = concept_rpc_codec::result_bytes(completion)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_concept_rpc_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_lease_identity(&recovery, &lease)?;
        check_lease(&transaction, &lease, now)?;
        let (continuation, retry_decision, backoff) = match completion.continuation {
            BoardContinuation::Retry { backoff_ms } => (
                "Retry",
                match completion.retry_decision {
                    crate::grpc_client::retry::RetryDecision::RetryBackoff => "RetryBackoff",
                    crate::grpc_client::retry::RetryDecision::RetryBounded => "RetryBounded",
                    crate::grpc_client::retry::RetryDecision::NoRetry => "NoRetry",
                },
                Some(backoff_ms),
            ),
            BoardContinuation::Terminal => (
                "Terminal",
                match completion.retry_decision {
                    crate::grpc_client::retry::RetryDecision::RetryBackoff => "RetryBackoff",
                    crate::grpc_client::retry::RetryDecision::RetryBounded => "RetryBounded",
                    crate::grpc_client::retry::RetryDecision::NoRetry => "NoRetry",
                },
                None,
            ),
        };
        let previous = lease.head;
        advance_run(&transaction, &mut lease, now, "concept RPC result cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_concept_rpc_attempt_results( \
                 intent_id,outer_ordinal,attempt_ordinal,begin_run_version,request_sha256, \
                 wire_outcome,result_codec_version,result_bytes,result_length,result_sha256, \
                 continuation,retry_decision,backoff_ms,run_id,run_context_sha256,input_sha256, \
                 lease_owner,lease_generation,prior_head_version,run_version,returned_at,committed_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,?9,?10,?11,?12,?13,?14,?15, \
                        ?16,?17,?18,?19,?20,?20)",
                params![
                    lease.intent_id.as_str(), call.ordinal, call.attempt, call.begin_version,
                    &call.request_digest,
                    if completion.response_bytes.is_some() { "Response" } else { "Status" },
                    &bytes, i64::try_from(bytes.len()).map_err(|_| storage("concept RPC result length"))?,
                    &digest, continuation, retry_decision, backoff,
                    recovery.context.run_id().as_str(), recovery.context.canonical_sha256().as_str(),
                    raw_digest(&recovery.input.encode()?).as_str(), lease.owner.as_str(),
                    lease.generation, previous, lease.head, now.get()
                ],
            )
            .map_err(|_| storage("concept RPC result fact"))?;
        let result_version = lease.head;
        let status_material = if completion.response_bytes.is_none() {
            let diagnostic = completion
                .processed
                .as_ref()
                .err()
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .safe_diagnostic();
            let material_bytes = board_error_codec::status_bytes(diagnostic)?;
            let material_digest = raw_digest(&material_bytes).as_str().to_owned();
            let previous = lease.head;
            advance_run(
                &transaction,
                &mut lease,
                now,
                "concept RPC status material cas",
            )?;
            transaction.execute(
                "INSERT INTO chain_post_close_concept_rpc_status_materials( \
                 intent_id,outer_ordinal,attempt_ordinal,result_run_version,result_sha256, \
                 request_sha256,provenance,projection_version,material_codec_version,material_bytes, \
                 material_length,material_sha256,run_id,run_context_sha256,input_sha256,lease_owner, \
                 lease_generation,prior_head_version,run_version,captured_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,'Captured',1,1,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![lease.intent_id.as_str(), call.ordinal, call.attempt, result_version,
                    &digest, &call.request_digest, &material_bytes,
                    i64::try_from(material_bytes.len()).map_err(|_| storage("concept RPC status length"))?,
                    &material_digest, recovery.context.run_id().as_str(),
                    recovery.context.canonical_sha256().as_str(), raw_digest(&recovery.input.encode()?).as_str(),
                    lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
            ).map_err(|_| storage("concept RPC status material fact"))?;
            Some((lease.head, material_digest))
        } else {
            None
        };
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        validate_concept_rpc_fact_rows(&transaction, &lease.intent_id)?;
        let stored = StoredAttemptResult {
            ordinal: call.ordinal,
            attempt: call.attempt,
            run_version: result_version,
            digest: digest.clone(),
            request_digest: call.request_digest,
            bytes,
            status_material,
        };
        let capability = (continuation == "Terminal").then(|| LiveErrorCapability {
            intent_id: lease.intent_id.clone(),
            run_id: lease.run_id.clone(),
            terminal: StoredAttemptResult {
                ordinal: stored.ordinal,
                attempt: stored.attempt,
                run_version: stored.run_version,
                digest,
                request_digest: stored.request_digest.clone(),
                bytes: stored.bytes.clone(),
                status_material: stored.status_material.clone(),
            },
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
            head: lease.head,
        });
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        Ok((lease, stored, capability))
    }

    pub(super) fn confirm_concept_rpc_error(
        &mut self,
        mut lease: RunLease,
        capability: LiveErrorCapability,
        error: &GatewayError,
        now: UtcMicros,
    ) -> Result<(RunLease, StoredErrorMaterial), ChainPostCloseError> {
        if capability.intent_id != lease.intent_id
            || capability.run_id != lease.run_id
            || capability.owner != lease.owner.as_str()
            || capability.generation != lease.generation
            || capability.head != lease.head
        {
            return Err(stale(&lease));
        }
        let code = occurrence_code(
            &self.store.connection,
            &lease.intent_id,
            capability.terminal.ordinal,
        )?;
        let request_hash = crate::data_gateway::review::board_membership_request_hash(&code);
        let observed_at = Utc
            .timestamp_micros(now.get())
            .single()
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let projected: Result<
            GatewayBatch<crate::data_gateway::BoardMembershipRecord>,
            GatewayError,
        > = Err(restore_gateway_error(&store_gateway_error(error))
            .map_err(|_| ChainPostCloseError::SchemaRejected)?);
        let audit = map_gateway_audit_record(
            "board-memberships",
            ProviderId::Tdx,
            &request_hash,
            &projected,
            &observed_at,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let bytes = board_error_codec::error_bytes(store_gateway_error(error), audit.clone())?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_concept_rpc_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_lease_identity(&recovery, &lease)?;
        check_lease(&transaction, &lease, now)?;
        if load_error_material(&transaction, &lease.intent_id, capability.terminal.ordinal)?
            .is_some()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let previous = lease.head;
        advance_run(
            &transaction,
            &mut lease,
            now,
            "concept RPC error material cas",
        )?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_concept_rpc_error_materials( \
             intent_id,outer_ordinal,terminal_attempt_ordinal,terminal_result_run_version, \
             terminal_result_sha256,request_sha256,status_material_attempt_ordinal, \
             status_material_run_version,status_material_sha256,material_codec_version, \
             material_bytes,material_length,material_sha256,observed_fallback,run_id, \
             run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version, \
             run_version,captured_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
                params![
                    lease.intent_id.as_str(),
                    capability.terminal.ordinal,
                    capability.terminal.attempt,
                    capability.terminal.run_version,
                    capability.terminal.digest,
                    capability.terminal.request_digest,
                    capability
                        .terminal
                        .status_material
                        .as_ref()
                        .map(|_| capability.terminal.attempt),
                    capability
                        .terminal
                        .status_material
                        .as_ref()
                        .map(|value| value.0),
                    capability
                        .terminal
                        .status_material
                        .as_ref()
                        .map(|value| value.1.as_str()),
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("concept RPC error length"))?,
                    &digest,
                    &observed_at,
                    recovery.context.run_id().as_str(),
                    recovery.context.canonical_sha256().as_str(),
                    raw_digest(&recovery.input.encode()?).as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get()
                ],
            )
            .map_err(|_| storage("concept RPC error material fact"))?;
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        validate_concept_rpc_fact_rows(&transaction, &lease.intent_id)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        let run_version = lease.head;
        Ok((
            lease,
            StoredErrorMaterial {
                run_version,
                digest,
                gateway: projected.unwrap_err(),
                audit,
            },
        ))
    }

    pub(super) fn finalize_concept_rpc(
        &mut self,
        mut lease: RunLease,
        terminal: &StoredAttemptResult,
        projected: &Result<GatewayBatch<crate::data_gateway::BoardMembershipRecord>, GatewayError>,
        material: Option<&StoredErrorMaterial>,
        now: UtcMicros,
    ) -> Result<(RunLease, StoredConceptProviderResult), ChainPostCloseError> {
        let code = occurrence_code(&self.store.connection, &lease.intent_id, terminal.ordinal)?;
        let observed_at = Utc
            .timestamp_micros(now.get())
            .single()
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let (final_outcome, raw, audit) = project_membership(
            &code,
            projected,
            material.map(|value| &value.audit),
            &observed_at,
        )?;
        let outer_outcome = if final_outcome == "Available" {
            "Returned"
        } else {
            "BusinessError"
        };
        let final_bytes = concept_rpc_codec::final_bytes(&final_outcome, raw.clone())?;
        let final_digest = raw_digest(&final_bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_concept_rpc_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_lease_identity(&recovery, &lease)?;
        check_lease(&transaction, &lease, now)?;
        let old_count: i64 = transaction.query_row(
            "SELECT (SELECT count(*) FROM chain_post_close_stage_begins WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=?2) + \
                    (SELECT count(*) FROM chain_post_close_stage_results WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=?2)",
            params![lease.intent_id.as_str(), terminal.ordinal], |row| row.get(0),
        ).map_err(|_| storage("concept RPC outer absence"))?;
        if old_count != 0
            || load_final(&transaction, &lease.intent_id, terminal.ordinal, &code)?.is_some()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let request_bytes = code.as_bytes();
        let request_digest = raw_digest(request_bytes).as_str().to_owned();
        let begin_previous = lease.head;
        advance_run(&transaction, &mut lease, now, "concept RPC outer begin cas")?;
        let outer_begin = lease.head;
        transaction.execute(
            "INSERT INTO chain_post_close_stage_begins(intent_id,effect_kind,effect_ordinal,effect_key,request_codec_version,request_bytes,request_length,request_sha256,lease_owner,lease_generation,run_version,begun_at) \
             VALUES(?1,'ConceptProvider',?2,?3,1,?4,?5,?6,?7,?8,?9,?10)",
            params![lease.intent_id.as_str(), terminal.ordinal, &code, request_bytes,
                i64::try_from(request_bytes.len()).map_err(|_| storage("concept RPC outer request length"))?,
                &request_digest, lease.owner.as_str(), lease.generation, lease.head, now.get()],
        ).map_err(|_| storage("concept RPC outer begin"))?;
        debug_assert_eq!(begin_previous + 1, outer_begin);
        let result_previous = lease.head;
        advance_run(
            &transaction,
            &mut lease,
            now,
            "concept RPC outer result cas",
        )?;
        let outer_result = lease.head;
        let raw_bytes = raw.as_bytes();
        let outer_result_digest = raw_digest(raw_bytes).as_str().to_owned();
        transaction.execute(
            "INSERT INTO chain_post_close_stage_results(intent_id,effect_kind,effect_ordinal,outcome,result_codec_version,result_bytes,result_length,result_sha256,lease_owner,lease_generation,run_version,returned_at,committed_at) \
             VALUES(?1,'ConceptProvider',?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?10)",
            params![lease.intent_id.as_str(), terminal.ordinal, outer_outcome, raw_bytes,
                i64::try_from(raw_bytes.len()).map_err(|_| storage("concept RPC outer result length"))?,
                &outer_result_digest, lease.owner.as_str(), lease.generation, lease.head, now.get()],
        ).map_err(|_| storage("concept RPC outer result"))?;
        debug_assert_eq!(result_previous + 1, outer_result);
        let borrowed = audit.borrowed("board-memberships");
        let receipt = append_acquisition_in_transaction(&transaction, &borrowed)
            .map_err(|_| storage("concept RPC audit append"))?;
        let final_previous = lease.head;
        advance_run(&transaction, &mut lease, now, "concept RPC final cas")?;
        transaction.execute(
            "INSERT INTO chain_post_close_concept_rpc_finals( \
             intent_id,outer_ordinal,code,provenance,occurrence_run_version,occurrence_request_sha256, \
             terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256, \
             error_material_run_version,error_material_sha256,final_outcome,final_codec_version, \
             final_bytes,final_length,final_sha256,outer_outcome,outer_begin_run_version, \
             outer_begin_sha256,outer_result_run_version,outer_result_sha256,audit_id,audit_record_hash, \
             previous_outcome,current_outcome,run_id,run_context_sha256,input_sha256,lease_owner, \
             lease_generation,prior_head_version,run_version,applied_at) \
             SELECT ?1,?2,?3,'CompatibilityProjection',occurrence.run_version,occurrence.request_sha256, \
                    ?4,?5,?6,?7,?8,?9,1,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21, \
                    run.run_id,run.run_context_sha256,run.input_sha256,?22,?23,?24,?25,?26 \
             FROM chain_post_close_concept_rpc_occurrences AS occurrence \
             JOIN chain_post_close_runs AS run ON run.intent_id=occurrence.intent_id \
             WHERE occurrence.intent_id=?1 AND occurrence.outer_ordinal=?2",
            params![lease.intent_id.as_str(), terminal.ordinal, &code, terminal.attempt,
                terminal.run_version, terminal.digest,
                material.map(|value| value.run_version), material.map(|value| value.digest.as_str()),
                final_outcome, &final_bytes,
                i64::try_from(final_bytes.len()).map_err(|_| storage("concept RPC final length"))?,
                &final_digest, outer_outcome, outer_begin, &request_digest, outer_result,
                &outer_result_digest, receipt.audit_id, receipt.record_hash,
                receipt.previous_outcome, receipt.current_outcome, lease.owner.as_str(),
                lease.generation, final_previous, lease.head, now.get()],
        ).map_err(|_| storage("concept RPC final fact"))?;
        verify_acquisition_receipt_in_transaction(&transaction, &receipt, &borrowed)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        validate_concept_rpc_fact_rows(&transaction, &lease.intent_id)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((
            lease,
            StoredConceptProviderResult {
                ordinal: terminal.ordinal,
                code,
                outcome: outer_outcome.to_owned(),
                bytes: raw.into_bytes(),
                run_version: outer_result,
            },
        ))
    }
}

struct Occurrence {
    run_version: u64,
    request_bytes: Vec<u8>,
    request_digest: String,
}

struct FinalFact {
    ordinal: u64,
    code: String,
    bytes: Vec<u8>,
    length: i64,
    digest: String,
    outcome: String,
    outer_outcome: String,
    terminal_bytes: Vec<u8>,
    request_bytes: Vec<u8>,
    outer_begin_bytes: Vec<u8>,
    outer_result_bytes: Vec<u8>,
    error_bytes: Option<Vec<u8>>,
    audit_id: i64,
    audit_record_hash: String,
    previous_outcome: Option<String>,
    current_outcome: String,
    acquisition_request_hash: String,
    audit_capability: String,
    audit_provider: String,
    audit_source: String,
    audit_request_hash: String,
    audit_source_at: Option<String>,
    audit_observed_at: String,
    audit_batch_id: Option<String>,
    audit_outcome: String,
    audit_request_count: i64,
    audit_accepted_count: i64,
    audit_rejected_count: i64,
    audit_reason_code: String,
    audit_retryable: bool,
    chain_record_hash: String,
    previous_provider_outcome: Option<String>,
}

fn occurrence_code(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
) -> Result<String, ChainPostCloseError> {
    connection
        .query_row(
            "SELECT code FROM chain_post_close_concept_rpc_occurrences \
             WHERE intent_id=?1 AND outer_ordinal=?2",
            params![intent.as_str(), ordinal],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| storage("concept RPC occurrence code read"))?
        .ok_or(ChainPostCloseError::SchemaRejected)
}

fn load_legacy_recovery(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
    code: &str,
) -> Result<Option<Recovery>, ChainPostCloseError> {
    let qualification = connection
        .query_row(
            "SELECT qualification_kind \
             FROM chain_post_close_concept_rpc_legacy_outer_qualifications \
             WHERE intent_id=?1 AND outer_ordinal=?2 AND code=?3",
            params![intent.as_str(), ordinal, code],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| storage("concept RPC legacy qualification read"))?;
    let Some(qualification) = qualification else {
        return Ok(None);
    };
    if qualification == "LegacyUnconfirmed" {
        return Ok(Some(Recovery::BegunUnconfirmed));
    }
    let result = super::load_result(connection, intent, ordinal)?
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let outcome_matches = match qualification.as_str() {
        "LegacyReturnedApplied" | "LegacyReturnedPendingCache" => result.outcome == "Returned",
        "LegacyBusinessError" => result.outcome == "BusinessError",
        _ => false,
    };
    if result.code != code || !outcome_matches {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(Some(Recovery::Complete(result)))
}

fn load_occurrence(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
    code: &str,
) -> Result<Option<Occurrence>, ChainPostCloseError> {
    connection.query_row(
        "SELECT run_version,CAST(request_bytes AS BLOB),request_sha256 FROM chain_post_close_concept_rpc_occurrences \
         WHERE intent_id=?1 AND outer_ordinal=?2 AND code=?3",
        params![intent.as_str(), ordinal, code],
        |row| Ok(Occurrence { run_version: row.get(0)?, request_bytes: row.get(1)?, request_digest: row.get(2)? }),
    ).optional().map_err(|_| storage("concept RPC occurrence read"))
}

fn load_attempt_result(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
    attempt: u32,
) -> Result<Option<StoredAttemptResult>, ChainPostCloseError> {
    connection.query_row(
        "SELECT run_version,result_sha256,request_sha256,CAST(result_bytes AS BLOB) FROM chain_post_close_concept_rpc_attempt_results \
         WHERE intent_id=?1 AND outer_ordinal=?2 AND attempt_ordinal=?3",
        params![intent.as_str(), ordinal, attempt],
        |row| Ok(StoredAttemptResult { ordinal, attempt, run_version: row.get(0)?, digest: row.get(1)?, request_digest: row.get(2)?, bytes: row.get(3)?, status_material: None }),
    ).optional().map_err(|_| storage("concept RPC result read"))
}

fn load_last_attempt(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
) -> Result<Option<(u32, u64, Option<StoredAttemptResult>)>, ChainPostCloseError> {
    let begun = connection.query_row(
        "SELECT attempt_ordinal,run_version FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=?1 AND outer_ordinal=?2 ORDER BY attempt_ordinal DESC LIMIT 1",
        params![intent.as_str(), ordinal], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?)),
    ).optional().map_err(|_| storage("concept RPC begin read"))?;
    begun
        .map(|(attempt, version)| {
            load_attempt_result(connection, intent, ordinal, attempt)
                .map(|result| (attempt, version, result))
        })
        .transpose()
}

fn load_error_material(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
) -> Result<Option<StoredErrorMaterial>, ChainPostCloseError> {
    connection.query_row(
        "SELECT run_version,material_sha256,CAST(material_bytes AS BLOB) FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=?1 AND outer_ordinal=?2",
        params![intent.as_str(), ordinal], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?)),
    ).optional().map_err(|_| storage("concept RPC error material read"))?.map(|(run_version, digest, bytes)| {
        if raw_digest(&bytes).as_str() != digest { return Err(ChainPostCloseError::SchemaRejected); }
        let (gateway, audit) = board_error_codec::decode_error(&bytes)?;
        Ok(StoredErrorMaterial { run_version, digest, gateway: restore_gateway_error(&gateway).map_err(|_| ChainPostCloseError::SchemaRejected)?, audit })
    }).transpose()
}

fn load_final(
    connection: &Connection,
    intent: &IntentId,
    ordinal: u64,
    code: &str,
) -> Result<Option<StoredConceptProviderResult>, ChainPostCloseError> {
    connection.query_row(
        "SELECT outer_outcome,CAST(final_bytes AS BLOB),outer_result_run_version FROM chain_post_close_concept_rpc_finals WHERE intent_id=?1 AND outer_ordinal=?2 AND code=?3",
        params![intent.as_str(), ordinal, code], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, u64>(2)?)),
    ).optional().map_err(|_| storage("concept RPC final read"))?.map(|(outcome, bytes, run_version)| {
        let (_, raw) = concept_rpc_codec::decode_final(&bytes)?;
        Ok(StoredConceptProviderResult { ordinal, code: code.to_owned(), outcome, bytes: raw.into_bytes(), run_version })
    }).transpose()
}

pub(super) fn validate_concept_rpc_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
) -> Result<(), ChainPostCloseError> {
    super::validate_owned_foreign_keys(transaction)?;
    validate_acquisition_chain_in_transaction(transaction)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    validate_concept_rpc_fact_rows(transaction, intent)
}

pub(super) fn validate_concept_rpc_fact_rows(
    connection: &Transaction<'_>,
    intent: &IntentId,
) -> Result<(), ChainPostCloseError> {
    let (run_id, context_digest, input_bytes, input_digest, owner, generation, head, updated_at): (
        String,
        String,
        Vec<u8>,
        String,
        String,
        i64,
        i64,
        i64,
    ) = connection
        .query_row(
            "SELECT run_id,run_context_sha256,CAST(input_bytes AS BLOB),input_sha256, \
                    lease_owner,lease_generation,head_version,updated_at \
             FROM chain_post_close_runs WHERE intent_id=?1",
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
                ))
            },
        )
        .map_err(|_| storage("concept RPC run facts"))?;
    let input = super::codec::FixedChainPreparationInput::decode(&input_bytes)?;
    if raw_digest(&input_bytes).as_str() != input_digest {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let missing = input
        .stocks()
        .iter()
        .filter_map(|stock| match input.cached_concepts(&stock.code) {
            Ok(None) => Some(Ok(stock.code.as_str())),
            Ok(Some(_)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let identity_mismatch: i64 = connection
        .query_row(
            "SELECT count(*) FROM ( \
               SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                      prior_head_version,run_version,planned_at AS fact_at \
                 FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=?1 \
               UNION ALL SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                      prior_head_version,run_version,begun_at \
                 FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=?1 \
               UNION ALL SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                      prior_head_version,run_version,committed_at \
                 FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=?1 \
               UNION ALL SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                      prior_head_version,run_version,captured_at \
                 FROM chain_post_close_concept_rpc_status_materials WHERE intent_id=?1 \
               UNION ALL SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                      prior_head_version,run_version,captured_at \
                 FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=?1 \
               UNION ALL SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                      prior_head_version,run_version,applied_at \
                 FROM chain_post_close_concept_rpc_finals WHERE intent_id=?1 \
             ) AS fact WHERE fact.run_id<>?2 OR fact.run_context_sha256<>?3 \
                OR fact.input_sha256<>?4 OR fact.lease_generation<1 \
                OR fact.lease_generation>?5 \
                OR (fact.lease_generation=?5 AND fact.lease_owner<>?6) \
                OR fact.prior_head_version+1<>fact.run_version \
                OR fact.run_version>?7 OR fact.fact_at>?8",
            params![
                intent.as_str(),
                &run_id,
                &context_digest,
                &input_digest,
                generation,
                &owner,
                head,
                updated_at
            ],
            |row| row.get(0),
        )
        .map_err(|_| storage("concept RPC identity facts"))?;
    if identity_mismatch != 0 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    validate_fact_relationships(connection, intent)?;
    validate_legacy_qualifications(connection, intent)?;
    let mut occurrence = connection
        .prepare(
            "SELECT outer_ordinal,code,request_id,request_codec_version,CAST(request_bytes AS BLOB), \
                    request_length,request_sha256,profile,acquisition_authority,retry_max_attempts, \
                    retry_base_delay_ms,retry_max_delay_ms,retry_jitter_ms, \
                    acquisition_request_hash,run_id,run_context_sha256,input_sha256,lease_owner, \
                    lease_generation,prior_head_version,run_version,planned_at \
             FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=?1 ORDER BY outer_ordinal",
        )
        .map_err(|_| storage("concept RPC facts"))?;
    let occurrences = occurrence
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, u32>(9)?,
                row.get::<_, u64>(10)?,
                row.get::<_, u64>(11)?,
                row.get::<_, u64>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
                row.get::<_, String>(15)?,
                row.get::<_, String>(16)?,
                row.get::<_, String>(17)?,
                row.get::<_, i64>(18)?,
                row.get::<_, i64>(19)?,
                row.get::<_, i64>(20)?,
                row.get::<_, i64>(21)?,
            ))
        })
        .map_err(|_| storage("concept RPC facts"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("concept RPC facts"))?;
    for (
        ordinal,
        code,
        request_id,
        codec,
        bytes,
        length,
        digest,
        profile,
        authority,
        max,
        base,
        cap,
        jitter,
        acquisition_hash,
        fact_run_id,
        fact_context,
        fact_input,
        fact_owner,
        fact_generation,
        prior_head,
        fact_version,
        planned_at,
    ) in occurrences
    {
        let decoded = concept_rpc_codec::decode_request(&bytes)?;
        if codec != 1
            || i64::try_from(bytes.len()).ok() != Some(length)
            || raw_digest(&bytes).as_str() != digest
            || decoded.code != code
            || decoded.request_id != request_id
            || match decoded.profile {
                crate::grpc_client::client::ContractProfile::LocalBridgeV1 => {
                    profile != "LocalBridgeV1"
                }
                crate::grpc_client::client::ContractProfile::ExternalV1 => profile != "ExternalV1",
            }
            || decoded.acquisition_authority != authority
            || decoded.retry_policy != (max, base, cap, jitter)
            || missing
                .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                .copied()
                != Some(code.as_str())
            || acquisition_hash != crate::data_gateway::review::board_membership_request_hash(&code)
            || fact_run_id != run_id
            || fact_context != context_digest
            || fact_input != input_digest
            || fact_generation < 1
            || fact_generation > generation
            || (fact_generation == generation && fact_owner != owner)
            || prior_head + 1 != fact_version
            || fact_version > head
            || planned_at > updated_at
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let (count, maximum): (i64, Option<i64>) = connection
            .query_row(
                "SELECT count(*),max(attempt_ordinal) FROM chain_post_close_concept_rpc_attempt_begins \
                 WHERE intent_id=?1 AND outer_ordinal=?2",
                params![intent.as_str(), ordinal],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| storage("concept RPC attempt prefix"))?;
        if count != maximum.unwrap_or(0) {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }
    let mut results = connection
        .prepare(
            "SELECT outer_ordinal,attempt_ordinal,wire_outcome,CAST(result_bytes AS BLOB), \
                    result_length,result_sha256,continuation \
             FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=?1 \
             ORDER BY outer_ordinal,attempt_ordinal",
        )
        .map_err(|_| storage("concept RPC results"))?;
    let results = results
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|_| storage("concept RPC results"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("concept RPC results"))?;
    for (ordinal, attempt, wire, bytes, length, digest, continuation) in results {
        if i64::try_from(bytes.len()).ok() != Some(length) || raw_digest(&bytes).as_str() != digest
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let decoded = concept_rpc_codec::decode_result(&bytes)?;
        match (wire.as_str(), continuation.as_str()) {
            ("Response", "Terminal") if decoded.terminal_response()?.is_some() => {}
            ("Status", "Retry") if decoded.confirmed_retry_backoff()?.is_some() => {}
            ("Status", "Terminal") if decoded.terminal_status()?.is_some() => {}
            _ => return Err(ChainPostCloseError::SchemaRejected),
        }
        let status_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_concept_rpc_status_materials \
                 WHERE intent_id=?1 AND outer_ordinal=?2 AND attempt_ordinal=?3",
                params![intent.as_str(), ordinal, attempt],
                |row| row.get(0),
            )
            .map_err(|_| storage("concept RPC status coverage"))?;
        let expected_status_count = if wire == "Status" { 1 } else { 0 };
        if status_count != expected_status_count {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }
    for (table, operation) in [
        (
            "chain_post_close_concept_rpc_status_materials",
            "concept RPC status facts",
        ),
        (
            "chain_post_close_concept_rpc_error_materials",
            "concept RPC error facts",
        ),
    ] {
        let sql = format!("SELECT CAST(material_bytes AS BLOB),material_length,material_sha256 FROM {table} WHERE intent_id=?1");
        let mut statement = connection.prepare(&sql).map_err(|_| storage(operation))?;
        let rows = statement
            .query_map([intent.as_str()], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|_| storage(operation))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| storage(operation))?;
        for (bytes, length, digest) in rows {
            if i64::try_from(bytes.len()).ok() != Some(length)
                || raw_digest(&bytes).as_str() != digest
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            if table.ends_with("status_materials") {
                board_error_codec::decode_status(&bytes)?;
            } else {
                let (gateway, _) = board_error_codec::decode_error(&bytes)?;
                restore_gateway_error(&gateway).map_err(|_| ChainPostCloseError::SchemaRejected)?;
            }
        }
    }
    let mut errors = connection
        .prepare(
            "SELECT CAST(error.material_bytes AS BLOB),error.observed_fallback, \
                    occurrence.acquisition_request_hash,terminal.wire_outcome, \
                    CAST(terminal.result_bytes AS BLOB),CAST(status.material_bytes AS BLOB), \
                    CAST(occurrence.request_bytes AS BLOB) \
             FROM chain_post_close_concept_rpc_error_materials AS error \
             JOIN chain_post_close_concept_rpc_occurrences AS occurrence \
               ON occurrence.intent_id=error.intent_id \
              AND occurrence.outer_ordinal=error.outer_ordinal \
             JOIN chain_post_close_concept_rpc_attempt_results AS terminal \
               ON terminal.intent_id=error.intent_id \
              AND terminal.outer_ordinal=error.outer_ordinal \
              AND terminal.attempt_ordinal=error.terminal_attempt_ordinal \
              AND terminal.run_version=error.terminal_result_run_version \
              AND terminal.result_sha256=error.terminal_result_sha256 \
             LEFT JOIN chain_post_close_concept_rpc_status_materials AS status \
               ON status.intent_id=error.intent_id \
              AND status.outer_ordinal=error.outer_ordinal \
              AND status.attempt_ordinal=error.status_material_attempt_ordinal \
              AND status.run_version=error.status_material_run_version \
              AND status.material_sha256=error.status_material_sha256 \
             WHERE error.intent_id=?1",
        )
        .map_err(|_| storage("concept RPC error mapping"))?;
    let errors = errors
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, Option<Vec<u8>>>(5)?,
                row.get::<_, Vec<u8>>(6)?,
            ))
        })
        .map_err(|_| storage("concept RPC error mapping"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("concept RPC error mapping"))?;
    for (bytes, fallback, request_hash, wire_outcome, result_bytes, status_bytes, request_bytes) in
        errors
    {
        let (stored_gateway, stored_audit) = board_error_codec::decode_error(&bytes)?;
        let request = concept_rpc_codec::decode_request(&request_bytes)?;
        let result = concept_rpc_codec::decode_result(&result_bytes)?;
        let projected = match wire_outcome.as_str() {
            "Status" => {
                let status = result
                    .terminal_status()?
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let diagnostic = status_bytes
                    .as_deref()
                    .ok_or(ChainPostCloseError::SchemaRejected)
                    .and_then(board_error_codec::decode_status)?;
                Err(ConnectedBoardQueries::restore_memberships_status(
                    request.profile,
                    &request.request_id,
                    status.code,
                    &status.details,
                    status.trailer.as_ref(),
                    diagnostic.as_deref(),
                )
                .ok_or(ChainPostCloseError::SchemaRejected)?)
            }
            "Response" => {
                if status_bytes.is_some() {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                let response = result
                    .terminal_response()?
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                ConnectedBoardQueries::restore_memberships_response(
                    request.profile,
                    request.acquisition_authority.as_deref(),
                    &request.request_id,
                    response,
                )
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        let projected_error = projected
            .as_ref()
            .err()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let expected = map_gateway_audit_record(
            "board-memberships",
            ProviderId::Tdx,
            &request_hash,
            &projected,
            &fallback,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if store_gateway_error(projected_error) != stored_gateway
            || restore_gateway_error(&stored_gateway).is_err()
            || expected != stored_audit
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }
    let mut finals = connection.prepare(
        "SELECT final.outer_ordinal,final.code,CAST(final.final_bytes AS BLOB),final.final_length, \
                final.final_sha256,final.final_outcome,final.outer_outcome, \
                CAST(terminal.result_bytes AS BLOB),CAST(occurrence.request_bytes AS BLOB), \
                CAST(outer_begin.request_bytes AS BLOB),CAST(outer_result.result_bytes AS BLOB), \
                CAST(error.material_bytes AS BLOB),final.audit_id,final.audit_record_hash, \
                final.previous_outcome,final.current_outcome,occurrence.acquisition_request_hash, \
                audit.capability,audit.provider,audit.source,audit.request_hash,audit.source_at, \
                audit.observed_at,audit.batch_id,audit.outcome,audit.request_count, \
                audit.accepted_count,audit.rejected_count,audit.reason_code,audit.retryable, \
                chain.record_hash, \
                (SELECT previous.outcome FROM data_acquisition_audit AS previous \
                  WHERE previous.id<audit.id AND previous.capability=audit.capability \
                    AND previous.provider=audit.provider ORDER BY previous.id DESC LIMIT 1) \
         FROM chain_post_close_concept_rpc_finals AS final \
         JOIN chain_post_close_concept_rpc_attempt_results AS terminal \
           ON terminal.intent_id=final.intent_id AND terminal.outer_ordinal=final.outer_ordinal \
          AND terminal.attempt_ordinal=final.terminal_attempt_ordinal \
          AND terminal.run_version=final.terminal_result_run_version \
          AND terminal.result_sha256=final.terminal_result_sha256 \
         JOIN chain_post_close_concept_rpc_occurrences AS occurrence \
           ON occurrence.intent_id=final.intent_id AND occurrence.outer_ordinal=final.outer_ordinal \
          AND occurrence.run_version=final.occurrence_run_version \
          AND occurrence.request_sha256=final.occurrence_request_sha256 \
         JOIN chain_post_close_stage_begins AS outer_begin \
           ON outer_begin.intent_id=final.intent_id AND outer_begin.effect_kind='ConceptProvider' \
          AND outer_begin.effect_ordinal=final.outer_ordinal \
          AND outer_begin.run_version=final.outer_begin_run_version \
          AND outer_begin.request_sha256=final.outer_begin_sha256 \
         JOIN chain_post_close_stage_results AS outer_result \
           ON outer_result.intent_id=final.intent_id AND outer_result.effect_kind='ConceptProvider' \
          AND outer_result.effect_ordinal=final.outer_ordinal \
          AND outer_result.run_version=final.outer_result_run_version \
          AND outer_result.result_sha256=final.outer_result_sha256 \
         LEFT JOIN chain_post_close_concept_rpc_error_materials AS error \
           ON error.intent_id=final.intent_id AND error.outer_ordinal=final.outer_ordinal \
          AND error.run_version=final.error_material_run_version \
          AND error.material_sha256=final.error_material_sha256 \
         JOIN data_acquisition_audit AS audit ON audit.id=final.audit_id \
         JOIN data_acquisition_audit_chain AS chain ON chain.acquisition_audit_id=audit.id \
         WHERE final.intent_id=?1 ORDER BY final.outer_ordinal",
    ).map_err(|_| storage("concept RPC finals"))?;
    let finals = finals
        .query_map([intent.as_str()], |row| {
            Ok(FinalFact {
                ordinal: row.get(0)?,
                code: row.get(1)?,
                bytes: row.get(2)?,
                length: row.get(3)?,
                digest: row.get(4)?,
                outcome: row.get(5)?,
                outer_outcome: row.get(6)?,
                terminal_bytes: row.get(7)?,
                request_bytes: row.get(8)?,
                outer_begin_bytes: row.get(9)?,
                outer_result_bytes: row.get(10)?,
                error_bytes: row.get(11)?,
                audit_id: row.get(12)?,
                audit_record_hash: row.get(13)?,
                previous_outcome: row.get(14)?,
                current_outcome: row.get(15)?,
                acquisition_request_hash: row.get(16)?,
                audit_capability: row.get(17)?,
                audit_provider: row.get(18)?,
                audit_source: row.get(19)?,
                audit_request_hash: row.get(20)?,
                audit_source_at: row.get(21)?,
                audit_observed_at: row.get(22)?,
                audit_batch_id: row.get(23)?,
                audit_outcome: row.get(24)?,
                audit_request_count: row.get(25)?,
                audit_accepted_count: row.get(26)?,
                audit_rejected_count: row.get(27)?,
                audit_reason_code: row.get(28)?,
                audit_retryable: row.get::<_, i64>(29)? != 0,
                chain_record_hash: row.get(30)?,
                previous_provider_outcome: row.get(31)?,
            })
        })
        .map_err(|_| storage("concept RPC finals"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("concept RPC finals"))?;
    for final_ in finals {
        let decoded = concept_rpc_codec::decode_final(&final_.bytes)?;
        let request = concept_rpc_codec::decode_request(&final_.request_bytes)?;
        let terminal = concept_rpc_codec::decode_result(&final_.terminal_bytes)?;
        let (expected_raw, expected_audit) = match final_.outcome.as_str() {
            "Available" | "VerifiedEmpty" => {
                let response = terminal
                    .terminal_response()?
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let projected = ConnectedBoardQueries::restore_memberships_response(
                    request.profile,
                    request.acquisition_authority.as_deref(),
                    &request.request_id,
                    response,
                )
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let expected_audit = map_gateway_audit_record(
                    "board-memberships",
                    projected.evidence().provider,
                    &final_.acquisition_request_hash,
                    &Ok(projected.clone()),
                    &final_.audit_observed_at,
                )
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let raw = match &projected {
                    GatewayBatch::Available { .. } if final_.outcome == "Available" => {
                        crate::agent::tools_sector::render_membership_batch(&final_.code, projected)
                            .map_err(|_| ChainPostCloseError::SchemaRejected)?
                    }
                    GatewayBatch::VerifiedEmpty(_) if final_.outcome == "VerifiedEmpty" => {
                        let error = crate::agent::tools_sector::render_membership_batch(
                            &final_.code,
                            projected,
                        )
                        .expect_err("verified empty membership is a business error");
                        crate::pipeline::chain_analysis::format_membership_fetch_failure(
                            &final_.code,
                            &error,
                        )
                    }
                    _ => return Err(ChainPostCloseError::SchemaRejected),
                };
                (raw, expected_audit)
            }
            "Error" => {
                let error_bytes = final_
                    .error_bytes
                    .as_deref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let (gateway, _) = board_error_codec::decode_error(&error_bytes)?;
                let (_, audit) = board_error_codec::decode_error(error_bytes)?;
                let gateway = restore_gateway_error(&gateway)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                (
                    crate::pipeline::chain_analysis::format_membership_fetch_failure(
                        &final_.code,
                        &gateway,
                    ),
                    audit,
                )
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        let expected_outer = if final_.outcome == "Available" {
            "Returned"
        } else {
            "BusinessError"
        };
        let expected = expected_audit.borrowed("board-memberships");
        if i64::try_from(final_.bytes.len()).ok() != Some(final_.length)
            || raw_digest(&final_.bytes).as_str() != final_.digest
            || decoded.0 != final_.outcome
            || decoded.1 != expected_raw
            || request.code != final_.code
            || final_.outer_outcome != expected_outer
            || final_.outer_begin_bytes != final_.code.as_bytes()
            || final_.outer_result_bytes != expected_raw.as_bytes()
            || final_.audit_capability != expected.capability
            || final_.audit_provider != expected.provider
            || final_.audit_source != expected.source
            || final_.audit_request_hash != expected.request_hash
            || final_.audit_source_at.as_deref() != expected.source_at
            || final_.audit_observed_at != expected.observed_at
            || final_.audit_batch_id.as_deref() != expected.batch_id
            || final_.audit_outcome != expected.outcome
            || final_.audit_request_count != expected.request_count
            || final_.audit_accepted_count != expected.accepted_count
            || final_.audit_rejected_count != expected.rejected_count
            || final_.audit_reason_code != expected.reason_code
            || final_.audit_retryable != expected.retryable
            || final_.audit_record_hash != final_.chain_record_hash
            || final_.previous_outcome != final_.previous_provider_outcome
            || final_.current_outcome != final_.audit_outcome
            || final_.audit_id < 1
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let receipt = DataAcquisitionAuditReceipt {
            audit_id: final_.audit_id,
            record_hash: final_.audit_record_hash.clone(),
            previous_outcome: final_.previous_outcome.clone(),
            current_outcome: final_.current_outcome.clone(),
        };
        verify_acquisition_receipt_in_transaction(connection, &receipt, &expected)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    }
    Ok(())
}

fn validate_fact_relationships(
    connection: &Connection,
    intent: &IntentId,
) -> Result<(), ChainPostCloseError> {
    let mismatch: i64 = connection
        .query_row(
            "SELECT count(*) FROM ( \
               SELECT begun.lease_generation AS child_generation, \
                      begun.lease_owner AS child_owner, \
                      occurrence.lease_generation AS parent_generation, \
                      occurrence.lease_owner AS parent_owner, \
                      begun.begun_at AS child_at,occurrence.planned_at AS parent_at \
                 FROM chain_post_close_concept_rpc_attempt_begins AS begun \
                 JOIN chain_post_close_concept_rpc_occurrences AS occurrence \
                   ON occurrence.intent_id=begun.intent_id \
                  AND occurrence.outer_ordinal=begun.outer_ordinal \
                WHERE begun.intent_id=?1 \
               UNION ALL \
               SELECT begun.lease_generation,begun.lease_owner, \
                      previous.lease_generation,previous.lease_owner, \
                      begun.begun_at,previous.committed_at \
                 FROM chain_post_close_concept_rpc_attempt_begins AS begun \
                 JOIN chain_post_close_concept_rpc_attempt_results AS previous \
                   ON previous.intent_id=begun.intent_id \
                  AND previous.outer_ordinal=begun.outer_ordinal \
                  AND previous.attempt_ordinal=begun.previous_attempt_ordinal \
                WHERE begun.intent_id=?1 AND begun.attempt_ordinal>1 \
               UNION ALL \
               SELECT result.lease_generation,result.lease_owner, \
                      begun.lease_generation,begun.lease_owner, \
                      result.returned_at,begun.begun_at \
                 FROM chain_post_close_concept_rpc_attempt_results AS result \
                 JOIN chain_post_close_concept_rpc_attempt_begins AS begun \
                   ON begun.intent_id=result.intent_id \
                  AND begun.outer_ordinal=result.outer_ordinal \
                  AND begun.attempt_ordinal=result.attempt_ordinal \
                WHERE result.intent_id=?1 \
               UNION ALL \
               SELECT status.lease_generation,status.lease_owner, \
                      result.lease_generation,result.lease_owner, \
                      status.captured_at,result.committed_at \
                 FROM chain_post_close_concept_rpc_status_materials AS status \
                 JOIN chain_post_close_concept_rpc_attempt_results AS result \
                   ON result.intent_id=status.intent_id \
                  AND result.outer_ordinal=status.outer_ordinal \
                  AND result.attempt_ordinal=status.attempt_ordinal \
                WHERE status.intent_id=?1 \
               UNION ALL \
               SELECT error.lease_generation,error.lease_owner, \
                      result.lease_generation,result.lease_owner, \
                      error.captured_at,result.committed_at \
                 FROM chain_post_close_concept_rpc_error_materials AS error \
                 JOIN chain_post_close_concept_rpc_attempt_results AS result \
                   ON result.intent_id=error.intent_id \
                  AND result.outer_ordinal=error.outer_ordinal \
                  AND result.attempt_ordinal=error.terminal_attempt_ordinal \
                WHERE error.intent_id=?1 \
               UNION ALL \
               SELECT final.lease_generation,final.lease_owner, \
                      terminal.lease_generation,terminal.lease_owner, \
                      final.applied_at,terminal.committed_at \
                 FROM chain_post_close_concept_rpc_finals AS final \
                 JOIN chain_post_close_concept_rpc_attempt_results AS terminal \
                   ON terminal.intent_id=final.intent_id \
                  AND terminal.outer_ordinal=final.outer_ordinal \
                  AND terminal.attempt_ordinal=final.terminal_attempt_ordinal \
                WHERE final.intent_id=?1 \
             ) AS relation \
             WHERE child_generation<parent_generation \
                OR (child_generation=parent_generation AND child_owner<>parent_owner) \
                OR child_at<parent_at",
            [intent.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage("concept RPC parent facts"))?;
    if mismatch != 0 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn validate_legacy_qualifications(
    connection: &Connection,
    intent: &IntentId,
) -> Result<(), ChainPostCloseError> {
    let mismatch: i64 = connection
        .query_row(
            "SELECT count(*) \
               FROM chain_post_close_concept_rpc_legacy_outer_qualifications AS legacy \
               LEFT JOIN chain_post_close_stage_begins AS begun \
                 ON begun.intent_id=legacy.intent_id \
                AND begun.effect_kind=legacy.effect_kind \
                AND begun.effect_ordinal=legacy.outer_ordinal \
               LEFT JOIN chain_post_close_stage_results AS result \
                 ON result.intent_id=legacy.intent_id \
                AND result.effect_kind=legacy.effect_kind \
                AND result.effect_ordinal=legacy.outer_ordinal \
               LEFT JOIN chain_post_close_concept_cache_writes AS cache \
                 ON cache.intent_id=legacy.intent_id \
                AND cache.effect_kind=legacy.effect_kind \
                AND cache.effect_ordinal=legacy.outer_ordinal \
              WHERE legacy.intent_id=?1 AND ( \
                    legacy.effect_kind<>'ConceptProvider' \
                 OR begun.intent_id IS NULL \
                 OR begun.effect_key<>legacy.code \
                 OR begun.run_version<>legacy.begin_run_version \
                 OR begun.request_sha256<>legacy.begin_request_sha256 \
                 OR begun.lease_owner<>legacy.begin_lease_owner \
                 OR begun.lease_generation<>legacy.begin_lease_generation \
                 OR begun.begun_at<>legacy.begun_at \
                 OR legacy.qualified_from_layout_version<>6 \
                 OR legacy.sealed_by_layout_version<>7 \
                 OR (legacy.qualification_kind='LegacyUnconfirmed' \
                     AND (result.intent_id IS NOT NULL OR cache.intent_id IS NOT NULL)) \
                 OR (legacy.qualification_kind='LegacyBusinessError' AND ( \
                        result.intent_id IS NULL OR result.outcome<>'BusinessError' \
                     OR result.run_version<>legacy.result_run_version \
                     OR result.result_sha256<>legacy.result_sha256 \
                     OR result.lease_owner<>legacy.result_lease_owner \
                     OR result.lease_generation<>legacy.result_lease_generation \
                     OR result.committed_at<>legacy.result_committed_at \
                     OR cache.intent_id IS NOT NULL)) \
                 OR (legacy.qualification_kind='LegacyReturnedPendingCache' AND ( \
                        result.intent_id IS NULL OR result.outcome<>'Returned' \
                     OR result.run_version<>legacy.result_run_version \
                     OR result.result_sha256<>legacy.result_sha256 \
                     OR result.lease_owner<>legacy.result_lease_owner \
                     OR result.lease_generation<>legacy.result_lease_generation \
                     OR result.committed_at<>legacy.result_committed_at \
                     OR (cache.intent_id IS NOT NULL AND (cache.code<>legacy.code \
                         OR cache.provider_result_run_version<>legacy.result_run_version)))) \
                 OR (legacy.qualification_kind='LegacyReturnedApplied' AND ( \
                        result.intent_id IS NULL OR result.outcome<>'Returned' \
                     OR result.run_version<>legacy.result_run_version \
                     OR result.result_sha256<>legacy.result_sha256 \
                     OR result.lease_owner<>legacy.result_lease_owner \
                     OR result.lease_generation<>legacy.result_lease_generation \
                     OR result.committed_at<>legacy.result_committed_at \
                     OR cache.intent_id IS NULL OR cache.code<>legacy.code \
                     OR cache.provider_result_run_version<>legacy.result_run_version \
                     OR cache.run_version<>legacy.cache_run_version \
                     OR cache.concepts_sha256<>legacy.cache_sha256 \
                     OR cache.lease_owner<>legacy.cache_lease_owner \
                     OR cache.lease_generation<>legacy.cache_lease_generation \
                     OR cache.written_at<>legacy.cache_written_at)) \
                 OR legacy.qualification_kind NOT IN ( \
                        'LegacyReturnedApplied','LegacyReturnedPendingCache', \
                        'LegacyBusinessError','LegacyUnconfirmed'))",
            [intent.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage("concept RPC legacy qualification facts"))?;
    if mismatch != 0 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn restored_request(
    request: concept_rpc_codec::RestoredRequest,
    next_attempt: u32,
) -> RestoredMembershipRequest {
    RestoredMembershipRequest::new(
        request.code,
        request.request_id,
        request.request,
        request.profile,
        request.acquisition_authority,
        request.retry_policy,
        next_attempt,
    )
}

fn validate_lease_identity(
    recovery: &super::RunRecovery,
    lease: &RunLease,
) -> Result<(), ChainPostCloseError> {
    if recovery.context.run_id() != &lease.run_id
        || recovery.input.encode()? != lease.input.encode()?
        || recovery.generation != lease.generation
        || recovery.head != lease.head
    {
        return Err(stale(lease));
    }
    Ok(())
}

fn stale(lease: &RunLease) -> ChainPostCloseError {
    ChainPostCloseError::StaleLease {
        intent_id: lease.intent_id.as_str().to_owned(),
    }
}

fn advance_run(
    connection: &Connection,
    lease: &mut RunLease,
    now: UtcMicros,
    operation: &'static str,
) -> Result<(), ChainPostCloseError> {
    let previous = lease.head;
    lease.head = previous
        .checked_add(1)
        .ok_or_else(|| storage("head overflow"))?;
    let changed = connection.execute(
        "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 WHERE intent_id=?3 AND lease_owner=?4 AND lease_generation=?5 AND head_version=?6 AND lease_until>?2",
        params![lease.head, now.get(), lease.intent_id.as_str(), lease.owner.as_str(), lease.generation, previous],
    ).map_err(|_| storage(operation))?;
    if changed != 1 {
        return Err(stale(lease));
    }
    Ok(())
}

/// Shared business projection; storage identities and transactions stay in their journals.
pub(super) fn project_membership(
    code: &str,
    projected: &Result<GatewayBatch<crate::data_gateway::BoardMembershipRecord>, GatewayError>,
    material_audit: Option<&OwnedGatewayAuditRecord>,
    observed_at: &str,
) -> Result<(String, String, OwnedGatewayAuditRecord), ChainPostCloseError> {
    let (outcome, raw) = match projected {
        Ok(batch @ GatewayBatch::Available { .. }) => (
            "Available",
            crate::agent::tools_sector::render_membership_batch(code, batch.clone())
                .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        ),
        Ok(batch @ GatewayBatch::VerifiedEmpty(_)) => {
            let error = crate::agent::tools_sector::render_membership_batch(code, batch.clone())
                .expect_err("verified empty membership is a business error");
            (
                "VerifiedEmpty",
                crate::pipeline::chain_analysis::format_membership_fetch_failure(code, &error),
            )
        }
        Err(error) => (
            "Error",
            crate::pipeline::chain_analysis::format_membership_fetch_failure(code, error),
        ),
    };
    let audit = match material_audit {
        Some(value) => value.clone(),
        None => map_gateway_audit_record(
            "board-memberships",
            projected
                .as_ref()
                .map_or(ProviderId::Tdx, |batch| batch.evidence().provider),
            &crate::data_gateway::review::board_membership_request_hash(code),
            projected,
            observed_at,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?,
    };
    Ok((outcome.to_owned(), raw, audit))
}
