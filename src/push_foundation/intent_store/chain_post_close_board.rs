use std::collections::BTreeMap;

use chrono::{SecondsFormat, TimeZone as _, Utc};
use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::data_gateway::grpc_source::{BoardAttemptCompletion, BoardContinuation};
use crate::data_gateway::review::{
    map_gateway_audit_record, restore_gateway_error, store_gateway_error, OwnedGatewayAuditRecord,
};
use crate::data_gateway::{
    BatchEvidence, BoardDirectoryFact, BoardKind, GatewayBatch, GatewayError,
};
use crate::database::data_acquisition_audit::{
    append_acquisition_in_transaction, validate_acquisition_chain_in_transaction,
    verify_acquisition_receipt_in_transaction, DataAcquisitionAuditReceipt,
    DataAcquisitionAuditRecord,
};
use crate::database::global_schema_catalog_v1::verify_br159_acquisition_catalog;
use crate::monitor::push_job::{raw_digest, IntentId, UtcMicros};

use super::board_codec;
use super::board_error_codec;
use super::cluster::{self, BoardParentFact};
use super::{
    check_lease, inspect_run_on, schema, storage, ChainPostCloseError, LocalChainPostClose,
    RunLease,
};

pub(super) const BOARD_LIMIT: u32 = 10_000;
const INDUSTRY_REQUEST_HASH: &str =
    "a60e8f768348b8c1cffcc9127857b61f109c7e5fc60f55fbddda98169d1d420f";
const CONCEPT_REQUEST_HASH: &str =
    "fe96ce4cfcabd4b4ca6fd5589792b7b1cb990ef3f445bdf688c6993950b0f6e5";

pub(super) struct BoardAttemptCall {
    kind: BoardKind,
    attempt_ordinal: u32,
    begin_run_version: u64,
    request_digest: String,
    owner: String,
    generation: u64,
}

pub(super) struct StoredBoardAttemptResult {
    kind: BoardKind,
    attempt_ordinal: u32,
    run_version: u64,
    digest: String,
}

pub(super) struct LiveBoardErrorCapability {
    intent_id: IntentId,
    terminal: StoredBoardAttemptResult,
    status_material: Option<(u64, String)>,
    owner: String,
    generation: u64,
    head: u64,
}

pub(crate) struct BoardErrorMaterialRecovery {
    gateway_error: GatewayError,
    audit: OwnedGatewayAuditRecord,
    status_diagnostic: Option<String>,
}

struct StoredBoardErrorMaterial {
    kind: BoardKind,
    recovery: BoardErrorMaterialRecovery,
    terminal_attempt: u32,
    terminal_result_version: u64,
    terminal_result_digest: String,
    request_digest: String,
    status_material_run_version: Option<u64>,
    status_material_sha256: Option<String>,
    run_id: String,
    context_digest: String,
    input_digest: String,
    owner: String,
    generation: u64,
    prior_head: u64,
    run_version: u64,
    captured_at: i64,
    bytes: Vec<u8>,
    digest: String,
}

impl BoardErrorMaterialRecovery {
    pub(crate) fn gateway_error(&self) -> &GatewayError {
        &self.gateway_error
    }

    pub(crate) fn audit_record(&self) -> DataAcquisitionAuditRecord<'_> {
        self.audit.borrowed("board-directory")
    }

    pub(crate) fn status_diagnostic(&self) -> Option<&str> {
        self.status_diagnostic.as_deref()
    }

    pub(super) fn into_gateway_error(self) -> GatewayError {
        self.gateway_error
    }
}

pub(super) enum PendingBoardAttempt {
    Response {
        terminal: StoredBoardAttemptResult,
        profile: crate::grpc_client::client::ContractProfile,
        acquisition_authority: Option<String>,
        request_id: String,
        response: crate::grpc_client::pb::magic::market::v1::QueryResponse,
    },
    Retry {
        profile: crate::grpc_client::client::ContractProfile,
        acquisition_authority: Option<String>,
        request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
        backoff_ms: u64,
    },
    Error {
        terminal: StoredBoardAttemptResult,
        material: BoardErrorMaterialRecovery,
    },
}

struct StoredBoardFinal {
    kind: BoardKind,
    application_version: u64,
    lifecycle_digest: String,
    industry_final_version: Option<u64>,
    industry_final_digest: Option<String>,
    terminal_attempt: u32,
    terminal_result_version: u64,
    terminal_result_digest: String,
    attempt_count: u32,
    outcome: String,
    run_version: u64,
    digest: String,
    bytes: Vec<u8>,
    receipt: DataAcquisitionAuditReceipt,
    run_id: String,
    context_digest: String,
    input_digest: String,
    owner: String,
    generation: u64,
    prior_head: u64,
    applied_at: i64,
}

struct LoadedBoardDirectory {
    run_version: u64,
    bytes: Vec<u8>,
    digest: String,
    fold_outcome: String,
}

struct LoadedBoardSelection {
    envelope: board_codec::SelectionEnvelope,
    bytes: Vec<u8>,
}

pub(super) struct PositionsParentFact {
    pub(super) owner: String,
    pub(super) generation: u64,
    pub(super) ready_at: i64,
    pub(super) run_version: u64,
}

/// Fact authority for one closed, no-write v12 inspection pass only.
/// It must not cross DML, a callback, await, or a subsequent inspection.
pub(super) struct ValidatedBoardFacts<'pass, 'connection> {
    transaction: &'pass Transaction<'connection>,
    run: &'pass super::RunRecovery,
    intent: String,
    layout: i64,
    parent: Option<&'pass BoardParentFact>,
}

impl ValidatedBoardFacts<'_, '_> {
    pub(super) fn check(
        &self,
        transaction: &Transaction<'_>,
        intent: &IntentId,
        run: &super::RunRecovery,
        parent: Option<&BoardParentFact>,
        layout: i64,
    ) -> Result<(), ChainPostCloseError> {
        let parent_matches = match (self.parent, parent) {
            (None, None) => true,
            (Some(expected), Some(actual)) => std::ptr::eq(expected, actual),
            _ => false,
        };
        if !std::ptr::eq(self.transaction, transaction)
            || !std::ptr::eq(self.run, run)
            || self.intent != intent.as_str()
            || self.layout != layout
            || !parent_matches
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(())
    }

    pub(super) fn positions_parent_for_read_pass(
        &self,
        transaction: &Transaction<'_>,
        intent: &IntentId,
        run: &super::RunRecovery,
        parent: Option<&BoardParentFact>,
        layout: i64,
    ) -> Result<PositionsParentFact, ChainPostCloseError> {
        self.check(transaction, intent, run, parent, layout)?;
        positions_parent_from_validated_facts(transaction, intent, parent)
    }
}

type LoadedBoardSelectionRow = (
    i64,
    String,
    String,
    Option<String>,
    i64,
    Vec<u8>,
    i64,
    String,
);

pub(crate) struct BoardDirectoryRecovery {
    attempts: Vec<BoardAttemptRecovery>,
    directories: Vec<BoardDirectoryKindRecovery>,
    board_directory: BTreeMap<String, String>,
    selected_board_codes: BTreeMap<String, String>,
    selection_fact_bytes: Vec<u8>,
}

pub(crate) struct BoardAttemptRecovery {
    kind: BoardKind,
    attempt_ordinal: u32,
    request_id: String,
    payload_bytes: Option<Vec<u8>>,
    error_detail_bytes: Option<Vec<u8>>,
    fact_bytes: Vec<u8>,
    confirmed: bool,
    continuation: Option<String>,
}

pub(crate) struct BoardDirectoryKindRecovery {
    kind: BoardKind,
    receipt: DataAcquisitionAuditReceipt,
    fact_bytes: Vec<u8>,
}

impl BoardDirectoryRecovery {
    pub(crate) fn attempts(&self) -> &[BoardAttemptRecovery] {
        &self.attempts
    }
    pub(crate) fn directories(&self) -> &[BoardDirectoryKindRecovery] {
        &self.directories
    }
    pub(crate) fn board_directory(&self) -> &BTreeMap<String, String> {
        &self.board_directory
    }
    pub(crate) fn selected_board_codes(&self) -> &BTreeMap<String, String> {
        &self.selected_board_codes
    }
    pub(crate) fn selection_fact_bytes(&self) -> &[u8] {
        &self.selection_fact_bytes
    }
}

impl BoardAttemptRecovery {
    pub(crate) fn kind(&self) -> BoardKind {
        self.kind
    }
    pub(crate) fn attempt_ordinal(&self) -> u32 {
        self.attempt_ordinal
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
    pub(crate) fn payload_bytes(&self) -> Option<&[u8]> {
        self.payload_bytes.as_deref()
    }
    pub(crate) fn error_detail_bytes(&self) -> Option<&[u8]> {
        self.error_detail_bytes.as_deref()
    }
    pub(crate) fn fact_bytes(&self) -> &[u8] {
        &self.fact_bytes
    }
    pub(crate) fn is_confirmed(&self) -> bool {
        self.confirmed
    }
}

impl BoardDirectoryKindRecovery {
    pub(crate) fn kind(&self) -> BoardKind {
        self.kind
    }
    pub(crate) fn receipt(&self) -> &DataAcquisitionAuditReceipt {
        &self.receipt
    }
    pub(crate) fn fact_bytes(&self) -> &[u8] {
        &self.fact_bytes
    }
}

impl LocalChainPostClose<'_> {
    pub(super) fn admit_board(
        &mut self,
        lease: &RunLease,
        now: UtcMicros,
    ) -> Result<(), ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        if recovery.context.run_id() != &lease.run_id
            || recovery.input.encode()? != lease.input.encode()?
            || recovery.generation != lease.generation
            || recovery.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        check_lease(&transaction, lease, now)?;
        verify_br159_acquisition_catalog(&transaction)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if layout_version < 7 {
            validate_acquisition_chain_in_transaction(&transaction)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        }
        transaction.commit().map_err(|_| storage("commit"))
    }

    pub(super) fn begin_board_attempt(
        &mut self,
        mut lease: RunLease,
        kind: BoardKind,
        authorized: &crate::grpc_client::client::board_attempt::AuthorizedBoardAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, BoardAttemptCall), ChainPostCloseError> {
        let request_bytes = board_codec::request_bytes(
            kind,
            authorized.request_id(),
            authorized.request_bytes(),
            authorized.profile(),
            authorized.acquisition_authority(),
            authorized.retry_policy(),
        )?;
        let request_digest = raw_digest(&request_bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_board_layout(&transaction)?;
        let recovery = validate_board_write(&transaction, &lease, now)?;
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if load_final(&transaction, &lease.intent_id, kind)?.is_some() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let attempt = authorized.attempt_ordinal();
        let previous_result = if attempt == 1 {
            None
        } else {
            load_result_identity(&transaction, &lease.intent_id, kind, attempt - 1)?
        };
        let industry = if kind == BoardKind::Concept {
            Some(
                load_final(&transaction, &lease.intent_id, BoardKind::Industry)?
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            )
        } else {
            None
        };
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "board begin cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_board_attempt_begins( \
                 intent_id,kind,attempt_ordinal,request_id,request_codec_version,request_bytes, \
                 request_length,request_sha256,previous_result_run_version,previous_result_sha256, \
                 industry_final_run_version,industry_final_sha256, \
                 chain_daily_application_run_version,lifecycle_sha256,run_id,run_context_sha256, \
                 input_sha256,lease_owner,lease_generation,prior_head_version,run_version,begun_at) \
                 VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
                params![
                    lease.intent_id.as_str(), board_codec::kind_name(kind), attempt,
                    authorized.request_id(), &request_bytes,
                    i64::try_from(request_bytes.len()).map_err(|_| storage("board request length"))?,
                    &request_digest,
                    previous_result.as_ref().map(|value| value.0),
                    previous_result.as_ref().map(|value| value.1.as_str()),
                    industry.as_ref().map(|value| value.run_version),
                    industry.as_ref().map(|value| value.digest.as_str()),
                    parent.application_version, parent.lifecycle_digest,
                    parent.run_id, parent.context_digest, parent.input_digest,
                    lease.owner.as_str(), lease.generation, previous, lease.head, now.get(),
                ],
            )
            .map_err(|_| storage("board begin fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        let call = BoardAttemptCall {
            kind,
            attempt_ordinal: attempt,
            begin_run_version: lease.head,
            request_digest,
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, call))
    }

    pub(super) fn record_board_attempt_result(
        &mut self,
        mut lease: RunLease,
        call: BoardAttemptCall,
        completion: &BoardAttemptCompletion,
        now: UtcMicros,
    ) -> Result<
        (
            RunLease,
            StoredBoardAttemptResult,
            Option<LiveBoardErrorCapability>,
        ),
        ChainPostCloseError,
    > {
        let bytes = board_codec::result_bytes(completion)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let (continuation, retry, backoff): (&str, &str, Option<u64>) =
            match completion.continuation {
                BoardContinuation::Retry { backoff_ms } => (
                    "Retry",
                    match completion.retry_decision {
                        crate::grpc_client::retry::RetryDecision::RetryBackoff => "RetryBackoff",
                        crate::grpc_client::retry::RetryDecision::RetryBounded => "RetryBounded",
                        crate::grpc_client::retry::RetryDecision::NoRetry => "NoRetry",
                    },
                    Some(backoff_ms),
                ),
                BoardContinuation::Terminal => ("Terminal", "NoRetry", None),
            };
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = validate_board_write(&transaction, &lease, now)?;
        cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if call.owner != lease.owner.as_str() || call.generation != lease.generation {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "board result cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_board_attempt_results( \
             intent_id,kind,attempt_ordinal,begin_run_version,request_sha256,wire_outcome, \
             result_codec_version,result_bytes,result_length,result_sha256,continuation, \
             retry_decision,backoff_ms,run_id,run_context_sha256,input_sha256,lease_owner, \
             lease_generation,prior_head_version,run_version,returned_at,committed_at) \
             VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?20)",
                params![
                    lease.intent_id.as_str(),
                    board_codec::kind_name(call.kind),
                    call.attempt_ordinal,
                    call.begin_run_version,
                    call.request_digest,
                    if completion.response_bytes.is_some() {
                        "Response"
                    } else {
                        "Status"
                    },
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("board result length"))?,
                    &digest,
                    continuation,
                    retry,
                    backoff,
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
            .map_err(|_| storage("board result fact"))?;
        let result_version = lease.head;
        let status_material = if layout_version >= 6 && completion.response_bytes.is_none() {
            let diagnostic = completion
                .processed
                .as_ref()
                .err()
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .safe_diagnostic();
            let material_bytes = board_error_codec::status_bytes(diagnostic)?;
            let material_digest = raw_digest(&material_bytes).as_str().to_owned();
            let previous = lease.head;
            lease.head = previous
                .checked_add(1)
                .ok_or_else(|| storage("head overflow"))?;
            advance_run(
                &transaction,
                &lease,
                previous,
                now,
                "board status material cas",
            )?;
            transaction
                .execute(
                    "INSERT INTO chain_post_close_board_status_materials( \
                     intent_id,kind,attempt_ordinal,result_run_version,result_sha256, \
                     request_sha256,provenance,projection_version,material_codec_version, \
                     material_bytes,material_length,material_sha256,run_id,run_context_sha256, \
                     input_sha256,lease_owner,lease_generation,prior_head_version,run_version, \
                     captured_at,legacy_layout_version) \
                     VALUES(?1,?2,?3,?4,?5,?6,'Captured',1,1,?7,?8,?9,?10,?11,?12, \
                            ?13,?14,?15,?16,?17,NULL)",
                    params![
                        lease.intent_id.as_str(),
                        board_codec::kind_name(call.kind),
                        call.attempt_ordinal,
                        result_version,
                        &digest,
                        &call.request_digest,
                        &material_bytes,
                        i64::try_from(material_bytes.len())
                            .map_err(|_| storage("board status material length"))?,
                        &material_digest,
                        recovery.context.run_id().as_str(),
                        recovery.context.canonical_sha256().as_str(),
                        raw_digest(&recovery.input.encode()?).as_str(),
                        lease.owner.as_str(),
                        lease.generation,
                        previous,
                        lease.head,
                        now.get(),
                    ],
                )
                .map_err(|_| storage("board status material fact"))?;
            Some((lease.head, material_digest))
        } else {
            None
        };
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        let result = StoredBoardAttemptResult {
            kind: call.kind,
            attempt_ordinal: call.attempt_ordinal,
            run_version: result_version,
            digest: digest.clone(),
        };
        let capability =
            (layout_version >= 6 && continuation == "Terminal").then(|| LiveBoardErrorCapability {
                intent_id: lease.intent_id.clone(),
                terminal: StoredBoardAttemptResult {
                    kind: call.kind,
                    attempt_ordinal: call.attempt_ordinal,
                    run_version: result_version,
                    digest,
                },
                status_material,
                owner: lease.owner.as_str().to_owned(),
                generation: lease.generation,
                head: lease.head,
            });
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, result, capability))
    }

    pub(super) fn confirm_board_error_material(
        &mut self,
        mut lease: RunLease,
        capability: LiveBoardErrorCapability,
        result: &Result<GatewayBatch<BoardDirectoryFact>, GatewayError>,
        now: UtcMicros,
    ) -> Result<(RunLease, BoardErrorMaterialRecovery), ChainPostCloseError> {
        let error = result
            .as_ref()
            .err()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if capability.intent_id != lease.intent_id
            || capability.owner != lease.owner.as_str()
            || capability.generation != lease.generation
            || capability.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        let observed_at = Utc
            .timestamp_micros(now.get())
            .single()
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let audit = crate::data_gateway::review::map_gateway_audit_record(
            "board-directory",
            crate::market_domain::ProviderId::Tdx,
            request_hash(capability.terminal.kind)?,
            result,
            &observed_at,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let bytes = board_error_codec::error_bytes(
            crate::data_gateway::review::store_gateway_error(error),
            audit.clone(),
        )?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        if verify_board_layout(&transaction)? < 6 {
            return Err(ChainPostCloseError::UnsupportedVersion);
        }
        let recovery = validate_board_write(&transaction, &lease, now)?;
        cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if load_error_material(&transaction, &lease.intent_id, capability.terminal.kind)?.is_some()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let (result_version, result_digest, request_digest, wire_outcome): (
            i64,
            String,
            String,
            String,
        ) = transaction
            .query_row(
                "SELECT run_version,result_sha256,request_sha256,wire_outcome \
                 FROM chain_post_close_board_attempt_results \
                 WHERE intent_id=?1 AND kind=?2 AND attempt_ordinal=?3",
                params![
                    lease.intent_id.as_str(),
                    board_codec::kind_name(capability.terminal.kind),
                    capability.terminal.attempt_ordinal,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|_| storage("board terminal result read"))?;
        if u64::try_from(result_version).ok() != Some(capability.terminal.run_version)
            || result_digest != capability.terminal.digest
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        match (&wire_outcome[..], &capability.status_material) {
            ("Status", Some(_)) | ("Response", None) => {}
            _ => return Err(ChainPostCloseError::SchemaRejected),
        }
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(
            &transaction,
            &lease,
            previous,
            now,
            "board error material cas",
        )?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_board_error_materials( \
                 intent_id,kind,terminal_attempt_ordinal,terminal_result_run_version, \
                 terminal_result_sha256,request_sha256,status_material_run_version, \
                 status_material_sha256,material_codec_version,material_bytes,material_length, \
                 material_sha256,run_id,run_context_sha256,input_sha256,lease_owner, \
                 lease_generation,prior_head_version,run_version,captured_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                params![
                    lease.intent_id.as_str(),
                    board_codec::kind_name(capability.terminal.kind),
                    capability.terminal.attempt_ordinal,
                    capability.terminal.run_version,
                    capability.terminal.digest,
                    request_digest,
                    capability.status_material.as_ref().map(|value| value.0),
                    capability
                        .status_material
                        .as_ref()
                        .map(|value| value.1.as_str()),
                    &bytes,
                    i64::try_from(bytes.len())
                        .map_err(|_| storage("board error material length"))?,
                    &digest,
                    recovery.context.run_id().as_str(),
                    recovery.context.canonical_sha256().as_str(),
                    raw_digest(&recovery.input.encode()?).as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("board error material fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        let stored = load_error_material(&transaction, &lease.intent_id, capability.terminal.kind)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let confirmed = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_error_material(&transaction, &lease.intent_id, &stored, &confirmed)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, stored.recovery))
    }

    pub(super) fn finalize_board_kind(
        &mut self,
        mut lease: RunLease,
        terminal: &StoredBoardAttemptResult,
        result: &Result<GatewayBatch<BoardDirectoryFact>, GatewayError>,
        now: UtcMicros,
    ) -> Result<
        (
            RunLease,
            Result<GatewayBatch<BoardDirectoryFact>, GatewayError>,
        ),
        ChainPostCloseError,
    > {
        let bytes = board_codec::batch_bytes(result)?;
        let decoded = board_codec::decode_batch(&bytes)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = validate_board_write(&transaction, &lease, now)?;
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if let Some(existing) = load_final(&transaction, &lease.intent_id, terminal.kind)? {
            validate_final(
                &transaction,
                &lease.intent_id,
                &existing,
                &recovery,
                &parent,
                layout_version,
            )?;
            let stored = final_result(
                &transaction,
                &lease.intent_id,
                &existing,
                &recovery,
                layout_version,
            )?;
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok((lease, stored));
        }
        let stored_result = load_result_identity(
            &transaction,
            &lease.intent_id,
            terminal.kind,
            terminal.attempt_ordinal,
        )?
        .ok_or(ChainPostCloseError::SchemaRejected)?;
        if stored_result.0 != terminal.run_version || stored_result.1 != terminal.digest {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let industry = if terminal.kind == BoardKind::Concept {
            Some(
                load_final(&transaction, &lease.intent_id, BoardKind::Industry)?
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            )
        } else {
            None
        };
        let observed_at = Utc
            .timestamp_micros(now.get())
            .single()
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let error_material = if layout_version >= 6 && result.is_err() {
            let stored = load_error_material(&transaction, &lease.intent_id, terminal.kind)?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            validate_error_material(&transaction, &lease.intent_id, &stored, &recovery)?;
            let result_error = result
                .as_ref()
                .err()
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            if store_gateway_error(result_error)
                != store_gateway_error(&stored.recovery.gateway_error)
                || stored.generation > lease.generation
                || (stored.generation == lease.generation && stored.owner != lease.owner.as_str())
                || stored.run_version > lease.head
                || stored.captured_at > now.get()
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Some(stored)
        } else {
            None
        };
        let audit = match &error_material {
            Some(material) => material.recovery.audit.clone(),
            None => {
                let provider = result
                    .as_ref()
                    .map_or(crate::market_domain::ProviderId::Tdx, |batch| {
                        batch.evidence().provider
                    });
                map_gateway_audit_record(
                    "board-directory",
                    provider,
                    request_hash(terminal.kind)?,
                    result,
                    &observed_at,
                )
                .map_err(|_| ChainPostCloseError::SchemaRejected)?
            }
        };
        let borrowed = audit.borrowed("board-directory");
        let receipt = append_acquisition_in_transaction(&transaction, &borrowed)
            .map_err(|_| storage("board audit append"))?;
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "board final cas")?;
        transaction.execute(
            "INSERT INTO chain_post_close_board_kind_finals( \
             intent_id,kind,chain_daily_application_run_version,lifecycle_sha256, \
             industry_final_run_version,industry_final_sha256,terminal_origin, \
             terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256, \
             attempt_count,final_outcome,final_codec_version,final_bytes,final_length,final_sha256, \
             audit_id,audit_record_hash,previous_outcome,current_outcome,run_id, \
             run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version, \
             run_version,applied_at) \
             VALUES(?1,?2,?3,?4,?5,?6,'Attempt',?7,?8,?9,?7,?10,1,?11,?12,?13, \
                    ?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
            params![lease.intent_id.as_str(), board_codec::kind_name(terminal.kind),
                parent.application_version, parent.lifecycle_digest,
                industry.as_ref().map(|value| value.run_version),
                industry.as_ref().map(|value| value.digest.as_str()),
                terminal.attempt_ordinal, terminal.run_version, terminal.digest,
                decoded.outcome_name(), &bytes,
                i64::try_from(bytes.len()).map_err(|_| storage("board final length"))?, &digest,
                receipt.audit_id, receipt.record_hash, receipt.previous_outcome,
                receipt.current_outcome, parent.run_id, parent.context_digest, parent.input_digest,
                lease.owner.as_str(), lease.generation, previous, lease.head, now.get()],
        ).map_err(|_| storage("board final fact"))?;
        crate::database::data_acquisition_audit::verify_acquisition_receipt_in_transaction(
            &transaction,
            &receipt,
            &borrowed,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let confirmed = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_fact_versions(&transaction, &lease.intent_id, confirmed.head)?;
        let confirmed_final = load_final(&transaction, &lease.intent_id, terminal.kind)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_final(
            &transaction,
            &lease.intent_id,
            &confirmed_final,
            &confirmed,
            &parent,
            layout_version,
        )?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((
            lease,
            match error_material {
                Some(material) => Err(material.recovery.gateway_error),
                None => decoded.into_result(),
            },
        ))
    }

    pub(super) fn load_board_kind(
        &mut self,
        lease: &RunLease,
        kind: BoardKind,
    ) -> Result<Option<Result<GatewayBatch<BoardDirectoryFact>, GatewayError>>, ChainPostCloseError>
    {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        let result = load_final(&transaction, &lease.intent_id, kind)?
            .map(|final_| {
                validate_final(
                    &transaction,
                    &lease.intent_id,
                    &final_,
                    &recovery,
                    &parent,
                    layout_version,
                )?;
                final_result(
                    &transaction,
                    &lease.intent_id,
                    &final_,
                    &recovery,
                    layout_version,
                )
            })
            .transpose()?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(result)
    }

    pub(super) fn load_pending_board_attempt(
        &mut self,
        lease: &RunLease,
        kind: BoardKind,
        now: UtcMicros,
    ) -> Result<Option<PendingBoardAttempt>, ChainPostCloseError> {
        struct Row {
            request_id: String,
            request_bytes: Vec<u8>,
            request_length: i64,
            request_digest: String,
            previous_result_version: Option<i64>,
            previous_result_digest: Option<String>,
            industry_final_version: Option<i64>,
            industry_final_digest: Option<String>,
            application_version: i64,
            lifecycle_digest: String,
            begin_run_id: String,
            begin_context_digest: String,
            begin_input_digest: String,
            begin_owner: String,
            begin_generation: i64,
            begin_prior: i64,
            begin_version: i64,
            begun_at: i64,
            result_begin_version: i64,
            result_request_digest: String,
            wire_outcome: String,
            result_bytes: Vec<u8>,
            result_length: i64,
            result_digest: String,
            continuation: String,
            retry_decision: String,
            backoff_ms: Option<i64>,
            result_run_id: String,
            result_context_digest: String,
            result_input_digest: String,
            result_owner: String,
            result_generation: i64,
            result_prior: i64,
            result_version: i64,
            returned_at: i64,
            committed_at: i64,
        }

        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = validate_board_write(&transaction, lease, now)?;
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        verify_br159_acquisition_catalog(&transaction)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if layout_version < 7 {
            validate_acquisition_chain_in_transaction(&transaction)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        }
        if load_final(&transaction, &lease.intent_id, kind)?.is_some() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let attempts = load_attempt_recovery(&transaction, &lease.intent_id)?;
        let matching = attempts
            .iter()
            .filter(|attempt| attempt.kind == kind)
            .collect::<Vec<_>>();
        let Some(last) = matching.last() else {
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(None);
        };
        if matching.iter().any(|attempt| !attempt.confirmed) {
            return Err(ChainPostCloseError::IncompleteEffect {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        if matching[..matching.len() - 1]
            .iter()
            .any(|attempt| attempt.continuation.as_deref() != Some("Retry"))
            || !matches!(last.continuation.as_deref(), Some("Retry" | "Terminal"))
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let row = transaction
            .query_row(
                "SELECT begin.request_id,begin.request_bytes,begin.request_length,begin.request_sha256, \
                        begin.previous_result_run_version,begin.previous_result_sha256, \
                        begin.industry_final_run_version,begin.industry_final_sha256, \
                        begin.chain_daily_application_run_version,begin.lifecycle_sha256, \
                        begin.run_id,begin.run_context_sha256,begin.input_sha256,begin.lease_owner, \
                        begin.lease_generation,begin.prior_head_version,begin.run_version,begin.begun_at, \
                        result.begin_run_version,result.request_sha256,result.wire_outcome, \
                        result.result_bytes,result.result_length,result.result_sha256,result.continuation, \
                        result.retry_decision,result.backoff_ms,result.run_id,result.run_context_sha256, \
                        result.input_sha256,result.lease_owner,result.lease_generation, \
                        result.prior_head_version,result.run_version,result.returned_at,result.committed_at \
                 FROM chain_post_close_board_attempt_begins AS begin \
                 JOIN chain_post_close_board_attempt_results AS result \
                   ON result.intent_id=begin.intent_id AND result.kind=begin.kind \
                  AND result.attempt_ordinal=begin.attempt_ordinal \
                 WHERE begin.intent_id=?1 AND begin.kind=?2 AND begin.attempt_ordinal=?3",
                params![
                    lease.intent_id.as_str(),
                    board_codec::kind_name(kind),
                    last.attempt_ordinal
                ],
                |row| {
                    Ok(Row {
                        request_id: row.get(0)?,
                        request_bytes: row.get(1)?,
                        request_length: row.get(2)?,
                        request_digest: row.get(3)?,
                        previous_result_version: row.get(4)?,
                        previous_result_digest: row.get(5)?,
                        industry_final_version: row.get(6)?,
                        industry_final_digest: row.get(7)?,
                        application_version: row.get(8)?,
                        lifecycle_digest: row.get(9)?,
                        begin_run_id: row.get(10)?,
                        begin_context_digest: row.get(11)?,
                        begin_input_digest: row.get(12)?,
                        begin_owner: row.get(13)?,
                        begin_generation: row.get(14)?,
                        begin_prior: row.get(15)?,
                        begin_version: row.get(16)?,
                        begun_at: row.get(17)?,
                        result_begin_version: row.get(18)?,
                        result_request_digest: row.get(19)?,
                        wire_outcome: row.get(20)?,
                        result_bytes: row.get(21)?,
                        result_length: row.get(22)?,
                        result_digest: row.get(23)?,
                        continuation: row.get(24)?,
                        retry_decision: row.get(25)?,
                        backoff_ms: row.get(26)?,
                        result_run_id: row.get(27)?,
                        result_context_digest: row.get(28)?,
                        result_input_digest: row.get(29)?,
                        result_owner: row.get(30)?,
                        result_generation: row.get(31)?,
                        result_prior: row.get(32)?,
                        result_version: row.get(33)?,
                        returned_at: row.get(34)?,
                        committed_at: row.get(35)?,
                    })
                },
            )
            .optional()
            .map_err(|_| storage("board pending response read"))?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let input_digest = raw_digest(&recovery.input.encode()?).as_str().to_owned();
        let previous_result = if last.attempt_ordinal == 1 {
            None
        } else {
            load_result_identity(
                &transaction,
                &lease.intent_id,
                kind,
                last.attempt_ordinal - 1,
            )?
        };
        let industry = if kind == BoardKind::Concept {
            let value = load_final(&transaction, &lease.intent_id, BoardKind::Industry)?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            Some((value.run_version, value.digest))
        } else {
            None
        };
        let begin_generation =
            u64::try_from(row.begin_generation).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let begin_version =
            u64::try_from(row.begin_version).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let result_generation = u64::try_from(row.result_generation)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let result_version =
            u64::try_from(row.result_version).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let historical_authority_valid = begin_generation > parent.application_generation
            || (begin_generation == parent.application_generation
                && row.begin_owner == parent.application_owner);
        let current_authority_valid = begin_generation < lease.generation
            || (begin_generation == lease.generation && row.begin_owner == lease.owner.as_str());
        if row.request_id != last.request_id
            || row.request_length != i64::try_from(row.request_bytes.len()).unwrap_or(-1)
            || row.request_digest != raw_digest(&row.request_bytes).as_str()
            || row.previous_result_version
                != previous_result
                    .as_ref()
                    .and_then(|value| i64::try_from(value.0).ok())
            || row.previous_result_digest.as_deref()
                != previous_result.as_ref().map(|value| value.1.as_str())
            || row.industry_final_version
                != industry
                    .as_ref()
                    .and_then(|value| i64::try_from(value.0).ok())
            || row.industry_final_digest.as_deref()
                != industry.as_ref().map(|value| value.1.as_str())
            || u64::try_from(row.application_version).ok() != Some(parent.application_version)
            || row.lifecycle_digest != parent.lifecycle_digest
            || row.begin_run_id != recovery.context.run_id().as_str()
            || row.begin_context_digest != recovery.context.canonical_sha256().as_str()
            || row.begin_input_digest != input_digest
            || row.begin_prior.checked_add(1) != Some(row.begin_version)
            || begin_version > recovery.head
            || row.begun_at < parent.applied_at
            || row.begun_at > recovery.updated_at
            || !historical_authority_valid
            || !current_authority_valid
            || row.result_begin_version != row.begin_version
            || row.result_request_digest != row.request_digest
            || row.result_length != i64::try_from(row.result_bytes.len()).unwrap_or(-1)
            || row.result_digest != raw_digest(&row.result_bytes).as_str()
            || row.result_run_id != row.begin_run_id
            || row.result_context_digest != row.begin_context_digest
            || row.result_input_digest != row.begin_input_digest
            || row.result_owner != row.begin_owner
            || result_generation != begin_generation
            || row.result_prior.checked_add(1) != Some(row.result_version)
            || row.result_prior != row.begin_version
            || result_version > recovery.head
            || row.returned_at < row.begun_at
            || row.committed_at < row.returned_at
            || row.committed_at > recovery.updated_at
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let request = board_codec::decode_request(&row.request_bytes)?;
        let restored_request = request.validate_for(kind, BOARD_LIMIT)?;
        if restored_request.request_id != row.request_id {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let result = board_codec::decode_result(&row.result_bytes)?;
        if row.continuation != result.continuation()
            || row.retry_decision != result.retry_decision()
            || row.backoff_ms.and_then(|value| u64::try_from(value).ok()) != result.backoff_ms()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let pending = match (
            row.wire_outcome.as_str(),
            result.terminal_response()?,
            result.confirmed_retry_backoff()?,
        ) {
            ("Response", Some(response), None) => {
                let terminal = StoredBoardAttemptResult {
                    kind,
                    attempt_ordinal: last.attempt_ordinal,
                    run_version: result_version,
                    digest: row.result_digest.clone(),
                };
                let projected =
                    crate::data_gateway::grpc_source::GrpcSource::restore_board_directory_response(
                        restored_request.profile,
                        restored_request.acquisition_authority.as_deref(),
                        &restored_request.request_id,
                        response.clone(),
                    );
                if projected.is_err() && layout_version >= 6 {
                    let material = load_error_material(&transaction, &lease.intent_id, kind)?
                        .ok_or_else(|| ChainPostCloseError::IncompleteEffect {
                            intent_id: lease.intent_id.as_str().to_owned(),
                        })?;
                    validate_error_material(&transaction, &lease.intent_id, &material, &recovery)?;
                    PendingBoardAttempt::Error {
                        terminal,
                        material: material.recovery,
                    }
                } else {
                    PendingBoardAttempt::Response {
                        terminal,
                        profile: restored_request.profile,
                        acquisition_authority: restored_request.acquisition_authority,
                        request_id: restored_request.request_id,
                        response,
                    }
                }
            }
            ("Status", None, Some(backoff_ms)) => {
                let next_attempt = last
                    .attempt_ordinal
                    .checked_add(1)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                if next_attempt > restored_request.retry_policy.0 {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                PendingBoardAttempt::Retry {
                    profile: restored_request.profile,
                    acquisition_authority: restored_request.acquisition_authority,
                    request: restored_request.request,
                    retry_policy: restored_request.retry_policy,
                    next_attempt,
                    backoff_ms,
                }
            }
            ("Status", None, None) if layout_version >= 6 => {
                let material = load_error_material(&transaction, &lease.intent_id, kind)?
                    .ok_or_else(|| ChainPostCloseError::IncompleteEffect {
                        intent_id: lease.intent_id.as_str().to_owned(),
                    })?;
                validate_error_material(&transaction, &lease.intent_id, &material, &recovery)?;
                PendingBoardAttempt::Error {
                    terminal: StoredBoardAttemptResult {
                        kind,
                        attempt_ordinal: last.attempt_ordinal,
                        run_version: result_version,
                        digest: row.result_digest,
                    },
                    material: material.recovery,
                }
            }
            ("Status", None, None) => {
                return Err(ChainPostCloseError::IncompleteEffect {
                    intent_id: lease.intent_id.as_str().to_owned(),
                });
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(Some(pending))
    }

    pub(super) fn load_board_directory(
        &mut self,
        lease: &RunLease,
    ) -> Result<Option<(BTreeMap<String, String>, Vec<BatchEvidence>)>, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        let Some(directory) = load_directory(&transaction, &lease.intent_id)? else {
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(None);
        };
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        let industry = load_final(&transaction, &lease.intent_id, BoardKind::Industry)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_final(
            &transaction,
            &lease.intent_id,
            &industry,
            &recovery,
            &parent,
            layout_version,
        )?;
        let concept = load_final(&transaction, &lease.intent_id, BoardKind::Concept)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_final(
            &transaction,
            &lease.intent_id,
            &concept,
            &recovery,
            &parent,
            layout_version,
        )?;
        let decoded = validate_directory_parent_content(
            &transaction,
            &lease.intent_id,
            &recovery,
            layout_version,
            Some(&industry),
            Some(&concept),
            &directory,
        )?;
        if directory.run_version > recovery.head
            || directory.digest != raw_digest(&directory.bytes).as_str()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let result = Some((
            decoded.codes,
            decoded
                .evidence
                .into_iter()
                .map(|value| value.into_batch())
                .collect(),
        ));
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(result)
    }

    pub(super) fn record_board_directory(
        &mut self,
        mut lease: RunLease,
        codes: &BTreeMap<String, String>,
        evidence: &[BatchEvidence],
        now: UtcMicros,
    ) -> Result<RunLease, ChainPostCloseError> {
        let bytes = board_codec::directory_bytes(codes, evidence)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_board_layout(&transaction)?;
        let recovery = validate_board_write(&transaction, &lease, now)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if load_directory(&transaction, &lease.intent_id)?.is_some() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let industry = load_final(&transaction, &lease.intent_id, BoardKind::Industry)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let concept = load_final(&transaction, &lease.intent_id, BoardKind::Concept)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "board directory cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_board_directory_materials( \
             intent_id,industry_final_run_version,industry_final_sha256,concept_final_run_version, \
             concept_final_sha256,fold_outcome,directory_codec_version,directory_bytes, \
             directory_length,directory_sha256,run_id,run_context_sha256,input_sha256,lease_owner, \
             lease_generation,prior_head_version,run_version,materialized_at) \
             VALUES(?1,?2,?3,?4,?5,'Available',1,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![
                    lease.intent_id.as_str(),
                    industry.run_version,
                    industry.digest,
                    concept.run_version,
                    concept.digest,
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("board directory length"))?,
                    &digest,
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
            .map_err(|_| storage("board directory fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(lease)
    }

    pub(super) fn record_board_selection(
        &mut self,
        mut lease: RunLease,
        ordinal: usize,
        concept: &str,
        selected_code: Option<&str>,
        now: UtcMicros,
    ) -> Result<(RunLease, Option<String>), ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_board_layout(&transaction)?;
        let recovery = validate_board_write(&transaction, &lease, now)?;
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if let Some(existing) = load_selection(&transaction, &lease.intent_id, ordinal)? {
            if existing.envelope.cluster_concept != concept {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok((lease, existing.envelope.selected_code));
        }
        let directory = load_directory(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let bytes = board_codec::selection_bytes(ordinal, concept, selected_code)?;
        let decoded = board_codec::decode_selection(&bytes)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "board selection cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_board_selections( \
             intent_id,cluster_ordinal,directory_run_version,directory_sha256, \
             cluster_material_run_version,cluster_material_sha256,cluster_concept, \
             selection_outcome,selected_code,selection_codec_version,selection_bytes, \
             selection_length,selection_sha256,run_id,run_context_sha256,input_sha256, \
             lease_owner,lease_generation,prior_head_version,run_version,selected_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
                params![
                    lease.intent_id.as_str(),
                    ordinal,
                    directory.run_version,
                    directory.digest,
                    parent.material_version,
                    parent.material_digest,
                    concept,
                    if selected_code.is_some() {
                        "Selected"
                    } else {
                        "NoMatch"
                    },
                    selected_code,
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("board selection length"))?,
                    digest,
                    parent.run_id,
                    parent.context_digest,
                    parent.input_digest,
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get()
                ],
            )
            .map_err(|_| storage("board selection fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, decoded.selected_code))
    }

    pub(super) fn load_board_selection(
        &mut self,
        lease: &RunLease,
        ordinal: usize,
        concept: &str,
    ) -> Result<Option<Option<String>>, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        let parent = cluster::load_board_parent(&transaction, &lease.intent_id, &recovery)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        let selection = load_selection(&transaction, &lease.intent_id, ordinal)?;
        if selection
            .as_ref()
            .is_some_and(|selection| selection.envelope.cluster_concept != concept)
            || parent
                .clusters
                .get(ordinal)
                .is_none_or(|cluster| cluster.concept != concept)
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        if let Some(selection) = &selection {
            let directory = load_directory(&transaction, &lease.intent_id)?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            let industry = load_final(&transaction, &lease.intent_id, BoardKind::Industry)?;
            let concept_final = load_final(&transaction, &lease.intent_id, BoardKind::Concept)?;
            for final_ in industry.iter().chain(concept_final.iter()) {
                validate_final(
                    &transaction,
                    &lease.intent_id,
                    final_,
                    &recovery,
                    &parent,
                    layout_version,
                )?;
            }
            let decoded = validate_directory_parent_content(
                &transaction,
                &lease.intent_id,
                &recovery,
                layout_version,
                industry.as_ref(),
                concept_final.as_ref(),
                &directory,
            )?;
            let board_map = decoded.codes.into_iter().collect();
            validate_one_selection_parent_content(&parent, &board_map, ordinal, selection)?;
        }
        let selected = selection.map(|selection| selection.envelope.selected_code);
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(selected)
    }

    pub(crate) fn inspect_board_error_material(
        &mut self,
        intent_id: &IntentId,
        kind: BoardKind,
    ) -> Result<Option<BoardErrorMaterialRecovery>, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        if verify_board_layout(&transaction)? < 6 {
            return Err(ChainPostCloseError::UnsupportedVersion);
        }
        let recovery = inspect_run_on(&transaction, intent_id)?;
        validate_fact_versions(&transaction, intent_id, recovery.head)?;
        let material = load_error_material(&transaction, intent_id, kind)?;
        if let Some(material) = &material {
            validate_error_material(&transaction, intent_id, material, &recovery)?;
        }
        let result = material.map(|material| material.recovery);
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(result)
    }

    pub(crate) fn inspect_board_directory(
        &mut self,
        intent_id: &IntentId,
    ) -> Result<BoardDirectoryRecovery, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let layout_version = verify_board_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, intent_id)?;
        let parent = cluster::load_board_parent(&transaction, intent_id, &recovery)?;
        validate_fact_versions(&transaction, intent_id, recovery.head)?;
        verify_br159_acquisition_catalog(&transaction)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if layout_version < 7 {
            validate_acquisition_chain_in_transaction(&transaction)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        }
        let attempts = load_attempt_recovery(&transaction, intent_id)?;
        let mut directories = Vec::new();
        for kind in [BoardKind::Industry, BoardKind::Concept] {
            let final_ = load_final(&transaction, intent_id, kind)?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            validate_final(
                &transaction,
                intent_id,
                &final_,
                &recovery,
                &parent,
                layout_version,
            )?;
            directories.push(BoardDirectoryKindRecovery {
                kind: final_.kind,
                receipt: final_.receipt,
                fact_bytes: final_.bytes,
            });
        }
        let directory =
            load_directory(&transaction, intent_id)?.ok_or(ChainPostCloseError::SchemaRejected)?;
        let industry = load_final(&transaction, intent_id, BoardKind::Industry)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let concept = load_final(&transaction, intent_id, BoardKind::Concept)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let decoded = validate_directory_parent_content(
            &transaction,
            intent_id,
            &recovery,
            layout_version,
            Some(&industry),
            Some(&concept),
            &directory,
        )?;
        let selections = load_selections(&transaction, intent_id)?;
        validate_selection_parent_content(&parent, &decoded, &selections)?;
        let selected_board_codes = selections
            .iter()
            .filter_map(|(_, value)| {
                value
                    .envelope
                    .selected_code
                    .clone()
                    .map(|code| (value.envelope.cluster_concept.clone(), code))
            })
            .collect();
        let selection_fact_bytes = selections
            .into_iter()
            .flat_map(|(_, value)| value.bytes)
            .collect();
        let result = BoardDirectoryRecovery {
            attempts,
            directories,
            board_directory: decoded.codes,
            selected_board_codes,
            selection_fact_bytes,
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(result)
    }
}

impl StoredBoardFinal {
    fn decoded(&self) -> Result<board_codec::BatchEnvelope, ChainPostCloseError> {
        if raw_digest(&self.bytes).as_str() != self.digest {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        board_codec::decode_batch(&self.bytes)
    }
}

fn request_hash(kind: BoardKind) -> Result<&'static str, ChainPostCloseError> {
    match kind {
        BoardKind::Industry => Ok(INDUSTRY_REQUEST_HASH),
        BoardKind::Concept => Ok(CONCEPT_REQUEST_HASH),
        BoardKind::Region => Err(ChainPostCloseError::SchemaRejected),
    }
}

fn verify_board_layout(connection: &Connection) -> Result<i64, ChainPostCloseError> {
    let version = schema::runtime_layout_version(connection)?;
    // runtime_layout_version has just attested this exact current catalog.
    if matches!(version, 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13) {
        Ok(version)
    } else {
        Err(ChainPostCloseError::UnsupportedVersion)
    }
}

fn validate_board_write(
    connection: &Transaction<'_>,
    lease: &RunLease,
    now: UtcMicros,
) -> Result<super::RunRecovery, ChainPostCloseError> {
    let recovery = inspect_run_on(connection, &lease.intent_id)?;
    if recovery.context.run_id() != &lease.run_id
        || recovery.input.encode()? != lease.input.encode()?
        || recovery.generation != lease.generation
        || recovery.head != lease.head
    {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    check_lease(connection, lease, now)?;
    Ok(recovery)
}

fn advance_run(
    connection: &Connection,
    lease: &RunLease,
    previous: u64,
    now: UtcMicros,
    operation: &'static str,
) -> Result<(), ChainPostCloseError> {
    let changed = connection
        .execute(
            "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
         WHERE intent_id=?3 AND run_id=?4 AND lease_owner=?5 AND lease_generation=?6 \
           AND head_version=?7 AND lease_until>?2",
            params![
                lease.head,
                now.get(),
                lease.intent_id.as_str(),
                lease.run_id.as_str(),
                lease.owner.as_str(),
                lease.generation,
                previous
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

fn load_result_identity(
    connection: &Connection,
    intent_id: &IntentId,
    kind: BoardKind,
    attempt: u32,
) -> Result<Option<(u64, String)>, ChainPostCloseError> {
    connection
        .query_row(
            "SELECT run_version,result_sha256 FROM chain_post_close_board_attempt_results \
         WHERE intent_id=?1 AND kind=?2 AND attempt_ordinal=?3",
            params![intent_id.as_str(), board_codec::kind_name(kind), attempt],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| storage("board result read"))?
        .map(|(version, digest)| {
            Ok((
                u64::try_from(version).map_err(|_| ChainPostCloseError::SchemaRejected)?,
                digest,
            ))
        })
        .transpose()
}

fn load_error_material(
    connection: &Connection,
    intent_id: &IntentId,
    kind: BoardKind,
) -> Result<Option<StoredBoardErrorMaterial>, ChainPostCloseError> {
    type Row = (
        i64,
        i64,
        String,
        String,
        Option<i64>,
        Option<String>,
        i64,
        Vec<u8>,
        i64,
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
    );
    connection
        .query_row(
            "SELECT terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256, \
                    request_sha256,status_material_run_version,status_material_sha256, \
                    material_codec_version,material_bytes,material_length,material_sha256, \
                    run_id,run_context_sha256, \
                    input_sha256,lease_owner,lease_generation,prior_head_version,run_version,captured_at \
             FROM chain_post_close_board_error_materials WHERE intent_id=?1 AND kind=?2",
            params![intent_id.as_str(), board_codec::kind_name(kind)],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                    row.get(15)?, row.get(16)?, row.get(17)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("board error material read"))?
        .map(|row: Row| {
            let (
                terminal_attempt,
                terminal_result_version,
                terminal_result_digest,
                request_digest,
                status_version,
                status_digest,
                codec,
                bytes,
                length,
                digest,
                run_id,
                context_digest,
                input_digest,
                owner,
                generation,
                prior_head,
                run_version,
                captured_at,
            ) = row;
            if codec != 1
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            let (gateway_stored, audit) = board_error_codec::decode_error(&bytes)?;
            let gateway_error = restore_gateway_error(&gateway_stored)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            let status_diagnostic = match (status_version, status_digest.as_deref()) {
                (Some(version), Some(expected_digest)) => {
                    let (status_bytes, status_length, actual_digest, provenance): (
                        Vec<u8>,
                        i64,
                        String,
                        String,
                    ) = connection
                        .query_row(
                            "SELECT material_bytes,material_length,material_sha256,provenance \
                             FROM chain_post_close_board_status_materials \
                             WHERE intent_id=?1 AND kind=?2 AND run_version=?3",
                            params![intent_id.as_str(), board_codec::kind_name(kind), version],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                        )
                        .map_err(|_| storage("board status material read"))?;
                    if provenance != "Captured"
                        || status_length != i64::try_from(status_bytes.len()).unwrap_or(-1)
                        || actual_digest != expected_digest
                        || actual_digest != raw_digest(&status_bytes).as_str()
                    {
                        return Err(ChainPostCloseError::SchemaRejected);
                    }
                    board_error_codec::decode_status(&status_bytes)?
                }
                (None, None) => None,
                _ => return Err(ChainPostCloseError::SchemaRejected),
            };
            Ok(StoredBoardErrorMaterial {
                kind,
                recovery: BoardErrorMaterialRecovery {
                    gateway_error,
                    audit,
                    status_diagnostic,
                },
                terminal_attempt: u32::try_from(terminal_attempt)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                terminal_result_version: u64::try_from(terminal_result_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                terminal_result_digest,
                request_digest,
                status_material_run_version: status_version
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                status_material_sha256: status_digest,
                run_id,
                context_digest,
                input_digest,
                owner,
                generation: u64::try_from(generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prior_head: u64::try_from(prior_head)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(run_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                captured_at,
                bytes,
                digest,
            })
        })
        .transpose()
}

fn validate_error_material(
    connection: &Connection,
    intent_id: &IntentId,
    material: &StoredBoardErrorMaterial,
    recovery: &super::RunRecovery,
) -> Result<(), ChainPostCloseError> {
    let (
        result_version,
        result_digest,
        request_digest,
        wire_outcome,
        committed_at,
        result_bytes,
        request_bytes,
    ): (
        i64,
        String,
        String,
        String,
        i64,
        Vec<u8>,
        Vec<u8>,
    ) = connection
        .query_row(
            "SELECT result.run_version,result.result_sha256,result.request_sha256, \
                    result.wire_outcome,result.committed_at,result.result_bytes,begin.request_bytes \
             FROM chain_post_close_board_attempt_results AS result \
             JOIN chain_post_close_board_attempt_begins AS begin \
               ON begin.intent_id=result.intent_id AND begin.kind=result.kind \
              AND begin.attempt_ordinal=result.attempt_ordinal \
             WHERE result.intent_id=?1 AND result.kind=?2 AND result.attempt_ordinal=?3 \
               AND result.continuation='Terminal'",
            params![
                intent_id.as_str(),
                board_codec::kind_name(material.kind),
                material.terminal_attempt,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .map_err(|_| storage("board error parent read"))?;
    let input_digest = raw_digest(&recovery.input.encode()?).as_str().to_owned();
    let authority_valid = material.generation < recovery.generation
        || (material.generation == recovery.generation
            && material.owner == recovery.owner.as_str());
    if u64::try_from(result_version).ok() != Some(material.terminal_result_version)
        || result_digest != material.terminal_result_digest
        || request_digest != material.request_digest
        || material.run_id != recovery.context.run_id().as_str()
        || material.context_digest != recovery.context.canonical_sha256().as_str()
        || material.input_digest != input_digest
        || material.prior_head.checked_add(1) != Some(material.run_version)
        || material.run_version > recovery.head
        || material.captured_at < committed_at
        || material.captured_at > recovery.updated_at
        || !authority_valid
        || (wire_outcome == "Status") != material.status_material_run_version.is_some()
        || material.status_material_run_version.is_some()
            != material.status_material_sha256.is_some()
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }

    if let (Some(status_version), Some(status_digest)) = (
        material.status_material_run_version,
        material.status_material_sha256.as_deref(),
    ) {
        type StatusRelation = (
            i64,
            i64,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            i64,
        );
        let status: StatusRelation = connection
            .query_row(
                "SELECT attempt_ordinal,result_run_version,result_sha256,request_sha256, \
                        run_id,run_context_sha256,input_sha256,lease_owner,lease_generation, \
                        prior_head_version,run_version,captured_at \
                 FROM chain_post_close_board_status_materials \
                 WHERE intent_id=?1 AND kind=?2 AND provenance='Captured' AND run_version=?3",
                params![
                    intent_id.as_str(),
                    board_codec::kind_name(material.kind),
                    status_version
                ],
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
                    ))
                },
            )
            .map_err(|_| storage("board status relation read"))?;
        let status_generation =
            u64::try_from(status.8).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let status_prior =
            u64::try_from(status.9).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let status_run =
            u64::try_from(status.10).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let status_authority_valid = status_generation < material.generation
            || (status_generation == material.generation && status.7 == material.owner);
        if u32::try_from(status.0).ok() != Some(material.terminal_attempt)
            || u64::try_from(status.1).ok() != Some(material.terminal_result_version)
            || status.2 != material.terminal_result_digest
            || status.3 != material.request_digest
            || status.4 != material.run_id
            || status.5 != material.context_digest
            || status.6 != material.input_digest
            || status_prior != material.terminal_result_version
            || status_prior.checked_add(1) != Some(status_run)
            || status_run != status_version
            || status_run >= material.run_version
            || status.11 < committed_at
            || status.11 > material.captured_at
            || !status_authority_valid
            || status_digest.len() != 64
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }

    let request = board_codec::decode_request(&request_bytes)?;
    let restored_request = request.validate_for(material.kind, BOARD_LIMIT)?;
    let result = board_codec::decode_result(&result_bytes)?;
    let projected = match wire_outcome.as_str() {
        "Status" => {
            let status = result
                .terminal_status()?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            Err(
                crate::data_gateway::grpc_source::GrpcSource::restore_board_directory_status(
                    restored_request.profile,
                    &restored_request.request_id,
                    status.code,
                    &status.details,
                    status.trailer.as_ref(),
                    material.recovery.status_diagnostic(),
                )
                .ok_or(ChainPostCloseError::SchemaRejected)?,
            )
        }
        "Response" => {
            let response = result
                .terminal_response()?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            crate::data_gateway::grpc_source::GrpcSource::restore_board_directory_response(
                restored_request.profile,
                restored_request.acquisition_authority.as_deref(),
                &restored_request.request_id,
                response,
            )
        }
        _ => return Err(ChainPostCloseError::SchemaRejected),
    };
    let projected_error = projected
        .as_ref()
        .err()
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let observed_at = Utc
        .timestamp_micros(material.captured_at)
        .single()
        .ok_or(ChainPostCloseError::SchemaRejected)?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let expected_audit = map_gateway_audit_record(
        "board-directory",
        crate::market_domain::ProviderId::Tdx,
        request_hash(material.kind)?,
        &projected,
        &observed_at,
    )
    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if store_gateway_error(projected_error) != store_gateway_error(&material.recovery.gateway_error)
        || expected_audit != material.recovery.audit
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn load_final(
    connection: &Connection,
    intent_id: &IntentId,
    kind: BoardKind,
) -> Result<Option<StoredBoardFinal>, ChainPostCloseError> {
    struct Row {
        kind: String,
        application_version: i64,
        lifecycle_digest: String,
        industry_final_version: Option<i64>,
        industry_final_digest: Option<String>,
        terminal_attempt: Option<i64>,
        terminal_result_version: Option<i64>,
        terminal_result_digest: Option<String>,
        attempt_count: i64,
        outcome: String,
        bytes: Vec<u8>,
        length: i64,
        digest: String,
        audit_id: i64,
        audit_record_hash: String,
        previous_outcome: Option<String>,
        current_outcome: String,
        run_id: String,
        context_digest: String,
        input_digest: String,
        owner: String,
        generation: i64,
        prior_head: i64,
        run_version: i64,
        applied_at: i64,
    }
    connection
        .query_row(
            "SELECT kind,chain_daily_application_run_version,lifecycle_sha256, \
                    industry_final_run_version,industry_final_sha256,terminal_attempt_ordinal, \
                    terminal_result_run_version,terminal_result_sha256,attempt_count,final_outcome, \
                    final_bytes,final_length,final_sha256,audit_id,audit_record_hash, \
                    previous_outcome,current_outcome,run_id,run_context_sha256,input_sha256, \
                    lease_owner,lease_generation,prior_head_version,run_version,applied_at \
             FROM chain_post_close_board_kind_finals WHERE intent_id=?1 AND kind=?2",
            params![intent_id.as_str(), board_codec::kind_name(kind)],
            |row| {
                Ok(Row {
                    kind: row.get(0)?,
                    application_version: row.get(1)?,
                    lifecycle_digest: row.get(2)?,
                    industry_final_version: row.get(3)?,
                    industry_final_digest: row.get(4)?,
                    terminal_attempt: row.get(5)?,
                    terminal_result_version: row.get(6)?,
                    terminal_result_digest: row.get(7)?,
                    attempt_count: row.get(8)?,
                    outcome: row.get(9)?,
                    bytes: row.get(10)?,
                    length: row.get(11)?,
                    digest: row.get(12)?,
                    audit_id: row.get(13)?,
                    audit_record_hash: row.get(14)?,
                    previous_outcome: row.get(15)?,
                    current_outcome: row.get(16)?,
                    run_id: row.get(17)?,
                    context_digest: row.get(18)?,
                    input_digest: row.get(19)?,
                    owner: row.get(20)?,
                    generation: row.get(21)?,
                    prior_head: row.get(22)?,
                    run_version: row.get(23)?,
                    applied_at: row.get(24)?,
                })
            },
        )
        .optional()
        .map_err(|_| storage("board final read"))?
        .map(|row| {
            let terminal_attempt = row
                .terminal_attempt
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            let terminal_result_version = row
                .terminal_result_version
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            let terminal_result_digest = row
                .terminal_result_digest
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            if row.length != i64::try_from(row.bytes.len()).unwrap_or(-1)
                || row.digest != raw_digest(&row.bytes).as_str()
                || row.attempt_count != terminal_attempt
                || row.run_version != row.prior_head.checked_add(1).unwrap_or(-1)
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            let parsed_kind = board_codec::parse_kind(&row.kind)?;
            if parsed_kind != kind {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(StoredBoardFinal {
                kind: parsed_kind,
                application_version: u64::try_from(row.application_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                lifecycle_digest: row.lifecycle_digest,
                industry_final_version: row
                    .industry_final_version
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                industry_final_digest: row.industry_final_digest,
                terminal_attempt: u32::try_from(terminal_attempt)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                terminal_result_version: u64::try_from(terminal_result_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                terminal_result_digest,
                attempt_count: u32::try_from(row.attempt_count)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                outcome: row.outcome,
                run_version: u64::try_from(row.run_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                digest: row.digest,
                bytes: row.bytes,
                receipt: DataAcquisitionAuditReceipt {
                    audit_id: row.audit_id,
                    record_hash: row.audit_record_hash,
                    previous_outcome: row.previous_outcome,
                    current_outcome: row.current_outcome,
                },
                run_id: row.run_id,
                context_digest: row.context_digest,
                input_digest: row.input_digest,
                owner: row.owner,
                generation: u64::try_from(row.generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prior_head: u64::try_from(row.prior_head)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                applied_at: row.applied_at,
            })
        })
        .transpose()
}

fn validate_final(
    transaction: &rusqlite::Transaction<'_>,
    intent_id: &IntentId,
    final_: &StoredBoardFinal,
    recovery: &super::RunRecovery,
    parent: &BoardParentFact,
    layout_version: i64,
) -> Result<(), ChainPostCloseError> {
    let decoded = final_.decoded()?;
    let stored_result =
        load_result_identity(transaction, intent_id, final_.kind, final_.terminal_attempt)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
    let expected_industry = if final_.kind == BoardKind::Concept {
        let industry = load_final(transaction, intent_id, BoardKind::Industry)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        Some((industry.run_version, industry.digest))
    } else {
        None
    };
    let authority_valid = final_.generation > parent.application_generation
        || (final_.generation == parent.application_generation
            && final_.owner == parent.application_owner);
    if final_.application_version != parent.application_version
        || final_.lifecycle_digest != parent.lifecycle_digest
        || final_.industry_final_version != expected_industry.as_ref().map(|value| value.0)
        || final_.industry_final_digest.as_deref()
            != expected_industry.as_ref().map(|value| value.1.as_str())
        || final_.terminal_result_version != stored_result.0
        || final_.terminal_result_digest != stored_result.1
        || final_.attempt_count != final_.terminal_attempt
        || final_.outcome != decoded.outcome_name()
        || final_.run_id != recovery.context.run_id().as_str()
        || final_.context_digest != recovery.context.canonical_sha256().as_str()
        || final_.input_digest != raw_digest(&recovery.input.encode()?).as_str()
        || !authority_valid
        || final_.prior_head.checked_add(1) != Some(final_.run_version)
        || final_.run_version > recovery.head
        || final_.applied_at < parent.applied_at
        || final_.applied_at > recovery.updated_at
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let attempt_count: i64 = transaction
        .query_row(
            "SELECT count(*) FROM chain_post_close_board_attempt_results \
             WHERE intent_id=?1 AND kind=?2",
            params![intent_id.as_str(), board_codec::kind_name(final_.kind)],
            |row| row.get(0),
        )
        .map_err(|_| storage("board final attempts"))?;
    if attempt_count != i64::from(final_.attempt_count) {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let result = decoded.into_result();
    let observed_at = Utc
        .timestamp_micros(final_.applied_at)
        .single()
        .ok_or(ChainPostCloseError::SchemaRejected)?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let expected_audit = if layout_version >= 6 && result.is_err() {
        let material =
            load_error_material(transaction, intent_id, final_.kind)?.ok_or_else(|| {
                ChainPostCloseError::IncompleteEffect {
                    intent_id: intent_id.as_str().to_owned(),
                }
            })?;
        validate_error_material(transaction, intent_id, &material, recovery)?;
        let material_authority_valid = material.generation < final_.generation
            || (material.generation == final_.generation && material.owner == final_.owner);
        let expected_error =
            restore_gateway_error(&store_gateway_error(&material.recovery.gateway_error))
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if !material_authority_valid
            || material.run_version >= final_.run_version
            || material.captured_at > final_.applied_at
            || board_codec::batch_bytes(&Err(expected_error))? != final_.bytes
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        material.recovery.audit
    } else {
        let provider = result
            .as_ref()
            .map_or(crate::market_domain::ProviderId::Tdx, |batch| {
                batch.evidence().provider
            });
        map_gateway_audit_record(
            "board-directory",
            provider,
            request_hash(final_.kind)?,
            &result,
            &observed_at,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
    };
    verify_acquisition_receipt_in_transaction(
        transaction,
        &final_.receipt,
        &expected_audit.borrowed("board-directory"),
    )
    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    Ok(())
}

fn final_result(
    connection: &Connection,
    intent_id: &IntentId,
    final_: &StoredBoardFinal,
    recovery: &super::RunRecovery,
    layout_version: i64,
) -> Result<Result<GatewayBatch<BoardDirectoryFact>, GatewayError>, ChainPostCloseError> {
    let decoded = final_.decoded()?;
    if layout_version >= 6 && decoded.outcome_name() == "Error" {
        let material =
            load_error_material(connection, intent_id, final_.kind)?.ok_or_else(|| {
                ChainPostCloseError::IncompleteEffect {
                    intent_id: intent_id.as_str().to_owned(),
                }
            })?;
        validate_error_material(connection, intent_id, &material, recovery)?;
        Ok(Err(material.recovery.gateway_error))
    } else {
        Ok(decoded.into_result())
    }
}

fn load_directory(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Option<LoadedBoardDirectory>, ChainPostCloseError> {
    connection
        .query_row(
            "SELECT run_version,directory_bytes,directory_length,directory_sha256,fold_outcome \
         FROM chain_post_close_board_directory_materials WHERE intent_id=?1",
            [intent_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("board directory read"))?
        .map(|(version, bytes, length, digest, fold_outcome)| {
            if length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || !matches!(fold_outcome.as_str(), "Available" | "Unavailable")
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            board_codec::decode_directory(&bytes)?;
            Ok(LoadedBoardDirectory {
                run_version: u64::try_from(version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                bytes,
                digest,
                fold_outcome,
            })
        })
        .transpose()
}

fn load_selection(
    connection: &Connection,
    intent_id: &IntentId,
    ordinal: usize,
) -> Result<Option<LoadedBoardSelection>, ChainPostCloseError> {
    connection
        .query_row(
            "SELECT cluster_ordinal,cluster_concept,selection_outcome,selected_code,selection_codec_version, \
                selection_bytes,selection_length,selection_sha256 \
         FROM chain_post_close_board_selections \
         WHERE intent_id=?1 AND cluster_ordinal=?2",
            params![intent_id.as_str(), ordinal],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("board selection read"))?
        .map(decode_loaded_selection)
        .transpose()
}

fn decode_loaded_selection(
    row: LoadedBoardSelectionRow,
) -> Result<LoadedBoardSelection, ChainPostCloseError> {
    let (ordinal, concept, outcome, selected_code, codec, bytes, length, digest) = row;
    let ordinal = usize::try_from(ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if codec != 1
        || length != i64::try_from(bytes.len()).unwrap_or(-1)
        || digest != raw_digest(&bytes).as_str()
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let selection = board_codec::decode_selection(&bytes)?;
    let expected_outcome = if selection.selected_code.is_some() {
        "Selected"
    } else {
        "NoMatch"
    };
    if selection.cluster_ordinal != ordinal
        || selection.cluster_concept != concept
        || selection.selected_code != selected_code
        || outcome != expected_outcome
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(LoadedBoardSelection {
        envelope: selection,
        bytes,
    })
}

fn load_selections(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Vec<(usize, LoadedBoardSelection)>, ChainPostCloseError> {
    let mut statement = connection
        .prepare(
            "SELECT cluster_ordinal,cluster_concept,selection_outcome,selected_code, \
                    selection_codec_version,selection_bytes,selection_length,selection_sha256 \
             FROM chain_post_close_board_selections \
             WHERE intent_id=?1 ORDER BY cluster_ordinal",
        )
        .map_err(|_| storage("board selections read"))?;
    let rows = statement
        .query_map([intent_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|_| storage("board selections read"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("board selections read"))?;
    rows.into_iter()
        .map(|row| {
            let selection = decode_loaded_selection(row)?;
            let ordinal = selection.envelope.cluster_ordinal;
            Ok((ordinal, selection))
        })
        .collect()
}

fn validate_directory_parent_content(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &super::RunRecovery,
    layout_version: i64,
    industry: Option<&StoredBoardFinal>,
    concept: Option<&StoredBoardFinal>,
    directory: &LoadedBoardDirectory,
) -> Result<board_codec::DirectoryEnvelope, ChainPostCloseError> {
    let decoded = board_codec::decode_directory(&directory.bytes)?;
    if directory.fold_outcome != "Available" {
        return Ok(decoded);
    }
    let industry = industry.ok_or(ChainPostCloseError::SchemaRejected)?;
    let concept = concept.ok_or(ChainPostCloseError::SchemaRejected)?;
    let mut codes = std::collections::HashMap::new();
    let mut evidence = Vec::new();
    for (kind, final_) in [
        (BoardKind::Industry, industry),
        (BoardKind::Concept, concept),
    ] {
        let batch = final_result(connection, intent_id, final_, recovery, layout_version)?
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        crate::pipeline::chain_analysis::fold_board_directory_kind(
            &mut codes,
            &mut evidence,
            kind,
            batch,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    }
    if codes.is_empty() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let canonical = codes.into_iter().collect::<BTreeMap<_, _>>();
    if board_codec::directory_bytes(&canonical, &evidence)?.as_slice() != directory.bytes.as_slice()
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(decoded)
}

fn validate_selection_parent_content(
    parent: &BoardParentFact,
    directory: &board_codec::DirectoryEnvelope,
    selections: &[(usize, LoadedBoardSelection)],
) -> Result<(), ChainPostCloseError> {
    let board_map = directory
        .codes
        .iter()
        .map(|(name, code)| (name.clone(), code.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    for (expected_ordinal, (ordinal, selection)) in selections.iter().enumerate() {
        if *ordinal != expected_ordinal {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        validate_one_selection_parent_content(parent, &board_map, *ordinal, selection)?;
    }
    Ok(())
}

fn validate_one_selection_parent_content(
    parent: &BoardParentFact,
    board_map: &std::collections::HashMap<String, String>,
    ordinal: usize,
    selection: &LoadedBoardSelection,
) -> Result<(), ChainPostCloseError> {
    let cluster = parent
        .clusters
        .get(ordinal)
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    if selection.envelope.cluster_concept != cluster.concept
        || !crate::pipeline::chain_analysis::stored_cluster_board_selection_is_valid(
            cluster,
            board_map,
            selection.envelope.selected_code.as_deref(),
        )
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn load_attempt_recovery(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Vec<BoardAttemptRecovery>, ChainPostCloseError> {
    struct StableRequest {
        next_ordinal: u32,
        request_id: String,
        bytes: Vec<u8>,
        digest: String,
    }

    type Row = (
        String,
        i64,
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        Option<i64>,
        Option<String>,
        Option<Vec<u8>>,
        Option<i64>,
        Option<String>,
    );
    let mut statement = connection.prepare(
        "SELECT begin.kind,begin.attempt_ordinal,begin.request_id,begin.run_version, \
                begin.request_bytes,begin.request_length,begin.request_sha256, \
                result.begin_run_version,result.request_sha256, \
                result.result_bytes,result.result_length,result.result_sha256 \
         FROM chain_post_close_board_attempt_begins AS begin \
         LEFT JOIN chain_post_close_board_attempt_results AS result \
           ON result.intent_id=begin.intent_id AND result.kind=begin.kind \
          AND result.attempt_ordinal=begin.attempt_ordinal \
         WHERE begin.intent_id=?1 ORDER BY CASE begin.kind WHEN 'Industry' THEN 0 ELSE 1 END,begin.attempt_ordinal"
    ).map_err(|_| storage("board attempts read"))?;
    let rows = statement
        .query_map([intent_id.as_str()], |row| {
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
            ))
        })
        .map_err(|_| storage("board attempts read"))?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .map_err(|_| storage("board attempts read"))?;
    let mut industry_request: Option<StableRequest> = None;
    let mut concept_request: Option<StableRequest> = None;
    rows.into_iter()
        .map(
            |(
                kind,
                ordinal,
                request_id,
                begin_version,
                request_bytes,
                request_length,
                request_digest,
                result_begin_version,
                result_request_digest,
                result_bytes,
                result_length,
                result_digest,
            )| {
                if request_length != i64::try_from(request_bytes.len()).unwrap_or(-1)
                    || request_digest != raw_digest(&request_bytes).as_str()
                {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                let request = board_codec::decode_request(&request_bytes)?;
                let ordinal =
                    u32::try_from(ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let parsed_kind = board_codec::parse_kind(&kind)?;
                if request.kind()? != parsed_kind
                    || request.request_id() != request_id
                    || request.request_wire().is_empty()
                {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                request.validate_for(parsed_kind, BOARD_LIMIT)?;
                let stable_request = match parsed_kind {
                    BoardKind::Industry => &mut industry_request,
                    BoardKind::Concept => &mut concept_request,
                    BoardKind::Region => return Err(ChainPostCloseError::SchemaRejected),
                };
                match stable_request {
                    Some(stable) => {
                        if ordinal != stable.next_ordinal
                            || request_id != stable.request_id
                            || request_bytes != stable.bytes
                            || request_digest != stable.digest
                        {
                            return Err(ChainPostCloseError::SchemaRejected);
                        }
                        stable.next_ordinal = stable
                            .next_ordinal
                            .checked_add(1)
                            .ok_or(ChainPostCloseError::SchemaRejected)?;
                    }
                    None => {
                        if ordinal != 1 {
                            return Err(ChainPostCloseError::SchemaRejected);
                        }
                        *stable_request = Some(StableRequest {
                            next_ordinal: 2,
                            request_id: request_id.clone(),
                            bytes: request_bytes.clone(),
                            digest: request_digest.clone(),
                        });
                    }
                }
                let (payload, error_detail, fact_bytes, confirmed, continuation) =
                    match result_bytes {
                        Some(result_bytes) => {
                            if result_begin_version != Some(begin_version)
                                || result_request_digest.as_deref() != Some(request_digest.as_str())
                                || result_length
                                    != Some(i64::try_from(result_bytes.len()).unwrap_or(-1))
                                || result_digest.as_deref()
                                    != Some(raw_digest(&result_bytes).as_str())
                            {
                                return Err(ChainPostCloseError::SchemaRejected);
                            }
                            let result = board_codec::decode_result(&result_bytes)?;
                            let mut fact = request_bytes.clone();
                            fact.extend_from_slice(&result_bytes);
                            (
                                result.payload_bytes()?,
                                result.error_detail_bytes().map(<[u8]>::to_vec),
                                fact,
                                true,
                                Some(result.continuation().to_owned()),
                            )
                        }
                        None => {
                            if result_begin_version.is_some()
                                || result_request_digest.is_some()
                                || result_length.is_some()
                                || result_digest.is_some()
                            {
                                return Err(ChainPostCloseError::SchemaRejected);
                            }
                            (None, None, request_bytes.clone(), false, None)
                        }
                    };
                Ok(BoardAttemptRecovery {
                    kind: parsed_kind,
                    attempt_ordinal: ordinal,
                    request_id: request.request_id().to_owned(),
                    payload_bytes: payload,
                    error_detail_bytes: error_detail,
                    fact_bytes,
                    confirmed,
                    continuation,
                })
            },
        )
        .collect()
}

fn validate_fact_versions(
    connection: &Connection,
    intent_id: &IntentId,
    head: u64,
) -> Result<(), ChainPostCloseError> {
    let layout_version = schema::runtime_layout_version(connection)?;
    super::validate_run_fact_versions(connection, intent_id, head)?;
    if layout_version >= 6 {
        validate_status_materials(connection, intent_id, head)?;
    }
    Ok(())
}

pub(super) fn validate_existing_board_facts_at_layout(
    transaction: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &super::RunRecovery,
    parent: Option<&BoardParentFact>,
    layout_version: i64,
) -> Result<(), ChainPostCloseError> {
    validate_existing_board_facts_scoped(
        transaction,
        intent_id,
        recovery,
        parent,
        layout_version,
        None,
    )
}

pub(super) fn validate_and_capture_board_facts_for_read_pass<'pass, 'connection>(
    transaction: &'pass Transaction<'connection>,
    intent: &IntentId,
    run: &'pass super::RunRecovery,
    parent: Option<&'pass BoardParentFact>,
    proof: &schema::V12CatalogProof<'_, '_>,
) -> Result<ValidatedBoardFacts<'pass, 'connection>, ChainPostCloseError> {
    validate_existing_board_facts_scoped(transaction, intent, run, parent, proof.layout(), Some(proof))?;
    Ok(ValidatedBoardFacts {
        transaction,
        run,
        intent: intent.as_str().to_owned(),
        layout: proof.layout(),
        parent,
    })
}

pub(super) fn validate_existing_board_facts_scoped(
    transaction: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &super::RunRecovery,
    parent: Option<&BoardParentFact>,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<(), ChainPostCloseError> {
    if proof.is_some() && layout_version < 12 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    if layout_version >= 12 {
        schema::verify_parent_layout_v12_scoped(transaction, proof)?;
    } else if !matches!(layout_version, 6 | 7 | 8 | 9 | 10 | 11) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let fact_count: i64 = transaction
        .query_row(
            "SELECT \
               (SELECT count(*) FROM chain_post_close_board_attempt_begins WHERE intent_id=?1)+ \
               (SELECT count(*) FROM chain_post_close_board_attempt_results WHERE intent_id=?1)+ \
               (SELECT count(*) FROM chain_post_close_board_status_materials WHERE intent_id=?1)+ \
               (SELECT count(*) FROM chain_post_close_board_error_materials WHERE intent_id=?1)+ \
               (SELECT count(*) FROM chain_post_close_board_kind_finals WHERE intent_id=?1)+ \
               (SELECT count(*) FROM chain_post_close_board_directory_materials WHERE intent_id=?1)+ \
               (SELECT count(*) FROM chain_post_close_board_selections WHERE intent_id=?1)",
            [intent_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage("board migration facts"))?;
    if fact_count == 0 {
        return Ok(());
    }
    let parent = parent.ok_or(ChainPostCloseError::SchemaRejected)?;
    let input_digest = raw_digest(&recovery.input.encode()?).as_str().to_owned();
    let identity_mismatch: i64 = transaction
        .query_row(
            "SELECT count(*) FROM ( \
               SELECT begun.run_id,begun.run_context_sha256,begun.input_sha256, \
                      begun.lease_owner,begun.lease_generation,begun.prior_head_version, \
                      begun.run_version,begun.begun_at AS fact_at, \
                      begun.chain_daily_application_run_version AS parent_version, \
                      begun.lifecycle_sha256 AS parent_sha256 \
                 FROM chain_post_close_board_attempt_begins AS begun WHERE begun.intent_id=?1 \
               UNION ALL \
               SELECT result.run_id,result.run_context_sha256,result.input_sha256, \
                      result.lease_owner,result.lease_generation,result.prior_head_version, \
                      result.run_version,result.committed_at, \
                      begun.chain_daily_application_run_version,begun.lifecycle_sha256 \
                 FROM chain_post_close_board_attempt_results AS result \
                 JOIN chain_post_close_board_attempt_begins AS begun \
                   ON begun.intent_id=result.intent_id AND begun.kind=result.kind \
                  AND begun.attempt_ordinal=result.attempt_ordinal \
                WHERE result.intent_id=?1 \
             ) AS fact WHERE fact.run_id<>?2 OR fact.run_context_sha256<>?3 \
                OR fact.input_sha256<>?4 OR fact.parent_version<>?5 OR fact.parent_sha256<>?6 \
                OR fact.lease_generation<?7 \
                OR (fact.lease_generation=?7 AND fact.lease_owner<>?8) \
                OR fact.lease_generation>?9 \
                OR (fact.lease_generation=?9 AND fact.lease_owner<>?10) \
                OR fact.prior_head_version+1<>fact.run_version \
                OR fact.run_version>?11 OR fact.fact_at<?12 OR fact.fact_at>?13",
            params![
                intent_id.as_str(),
                recovery.context.run_id().as_str(),
                recovery.context.canonical_sha256().as_str(),
                input_digest,
                parent.application_version,
                parent.lifecycle_digest,
                parent.application_generation,
                parent.application_owner,
                recovery.generation,
                recovery.owner,
                recovery.head,
                parent.applied_at,
                recovery.updated_at,
            ],
            |row| row.get(0),
        )
        .map_err(|_| storage("board migration identity facts"))?;
    if identity_mismatch != 0 {
        return Err(ChainPostCloseError::SchemaRejected);
    }

    let attempts = load_attempt_recovery(transaction, intent_id)?;
    for kind in [BoardKind::Industry, BoardKind::Concept] {
        let matching = attempts
            .iter()
            .filter(|attempt| attempt.kind == kind)
            .collect::<Vec<_>>();
        if let Some((last, previous)) = matching.split_last() {
            if previous.iter().any(|attempt| {
                !attempt.confirmed || attempt.continuation.as_deref() != Some("Retry")
            }) || (last.confirmed
                && !matches!(last.continuation.as_deref(), Some("Retry" | "Terminal")))
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
        }
    }
    validate_status_materials(transaction, intent_id, recovery.head)?;

    for kind in [BoardKind::Industry, BoardKind::Concept] {
        if let Some(material) = load_error_material(transaction, intent_id, kind)? {
            validate_error_material(transaction, intent_id, &material, recovery)?;
        }
    }
    let industry = load_final(transaction, intent_id, BoardKind::Industry)?;
    let concept = load_final(transaction, intent_id, BoardKind::Concept)?;
    if let Some(final_) = &industry {
        validate_final(
            transaction,
            intent_id,
            final_,
            recovery,
            parent,
            layout_version,
        )?;
    }
    if let Some(final_) = &concept {
        if industry.is_none() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        validate_final(
            transaction,
            intent_id,
            final_,
            recovery,
            parent,
            layout_version,
        )?;
    }

    let directory = load_directory(transaction, intent_id)?;
    let decoded_directory = if let Some(directory) = &directory {
        let industry = industry
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        type DirectoryIdentity = (
            i64,
            String,
            Option<i64>,
            Option<String>,
            String,
            String,
            String,
            String,
            String,
            i64,
            i64,
            i64,
        );
        let identity: DirectoryIdentity = transaction
            .query_row(
                "SELECT industry_final_run_version,industry_final_sha256, \
                        concept_final_run_version,concept_final_sha256,fold_outcome,run_id, \
                        run_context_sha256,input_sha256,lease_owner,lease_generation, \
                        prior_head_version,materialized_at \
                 FROM chain_post_close_board_directory_materials WHERE intent_id=?1",
                [intent_id.as_str()],
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
                    ))
                },
            )
            .map_err(|_| storage("board directory migration facts"))?;
        let stored_concept = match (&concept, identity.4.as_str()) {
            (Some(final_), _) => Some((final_.run_version, final_.digest.as_str())),
            (None, "Unavailable") => None,
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        if u64::try_from(identity.0).ok() != Some(industry.run_version)
            || identity.1 != industry.digest
            || identity.2.and_then(|value| u64::try_from(value).ok())
                != stored_concept.map(|value| value.0)
            || identity.3.as_deref() != stored_concept.map(|value| value.1)
            || !matches!(identity.4.as_str(), "Available" | "Unavailable")
            || identity.5 != recovery.context.run_id().as_str()
            || identity.6 != recovery.context.canonical_sha256().as_str()
            || identity.7 != input_digest
            || identity.9 < i64::try_from(parent.application_generation).unwrap_or(i64::MAX)
            || (identity.9 == i64::try_from(parent.application_generation).unwrap_or(-1)
                && identity.8 != parent.application_owner)
            || identity.9 > i64::try_from(recovery.generation).unwrap_or(-1)
            || (identity.9 == i64::try_from(recovery.generation).unwrap_or(-1)
                && identity.8 != recovery.owner)
            || identity.10.checked_add(1) != i64::try_from(directory.run_version).ok()
            || identity.11 > recovery.updated_at
            || raw_digest(&directory.bytes).as_str() != directory.digest
            || directory.run_version > recovery.head
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Some(validate_directory_parent_content(
            transaction,
            intent_id,
            recovery,
            layout_version,
            Some(industry),
            concept.as_ref(),
            directory,
        )?)
    } else {
        None
    };

    let selections = load_selections(transaction, intent_id)?;
    if !selections.is_empty() && directory.is_none() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    for (ordinal, selection) in &selections {
        let cluster = parent
            .clusters
            .get(*ordinal)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if selection.envelope.cluster_concept != cluster.concept {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let directory = directory
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let mismatch: i64 = transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_board_selections \
                 WHERE intent_id=?1 AND cluster_ordinal=?2 AND (directory_run_version<>?3 \
                    OR directory_sha256<>?4 OR cluster_material_run_version<>?5 \
                    OR cluster_material_sha256<>?6 OR run_id<>?7 \
                    OR run_context_sha256<>?8 OR input_sha256<>?9 \
                    OR prior_head_version+1<>run_version OR run_version>?10 \
                    OR selected_at>?11 OR lease_generation<?12 \
                    OR (lease_generation=?12 AND lease_owner<>?13) \
                    OR lease_generation>?14 \
                    OR (lease_generation=?14 AND lease_owner<>?15))",
                params![
                    intent_id.as_str(),
                    *ordinal,
                    directory.run_version,
                    directory.digest,
                    parent.material_version,
                    parent.material_digest,
                    recovery.context.run_id().as_str(),
                    recovery.context.canonical_sha256().as_str(),
                    input_digest,
                    recovery.head,
                    recovery.updated_at,
                    parent.application_generation,
                    parent.application_owner,
                    recovery.generation,
                    recovery.owner,
                ],
                |row| row.get(0),
            )
            .map_err(|_| storage("board selection migration facts"))?;
        if mismatch != 0 {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }
    if let Some(decoded) = &decoded_directory {
        validate_selection_parent_content(parent, decoded, &selections)?;
    }
    Ok(())
}

pub(super) fn validate_positions_parent_at_layout(
    transaction: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &super::RunRecovery,
    parent: Option<&BoardParentFact>,
    layout_version: i64,
) -> Result<PositionsParentFact, ChainPostCloseError> {
    validate_positions_parent_scoped(
        transaction,
        intent_id,
        recovery,
        parent,
        layout_version,
        None,
    )
}

pub(super) fn validate_positions_parent_scoped(
    transaction: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &super::RunRecovery,
    parent: Option<&BoardParentFact>,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<PositionsParentFact, ChainPostCloseError> {
    validate_existing_board_facts_scoped(
        transaction,
        intent_id,
        recovery,
        parent,
        layout_version,
        proof,
    )?;
    positions_parent_from_validated_facts(transaction, intent_id, parent)
}

fn positions_parent_from_validated_facts(
    transaction: &Transaction<'_>,
    intent_id: &IntentId,
    parent: Option<&BoardParentFact>,
) -> Result<PositionsParentFact, ChainPostCloseError> {
    let parent = parent.ok_or(ChainPostCloseError::SchemaRejected)?;
    let directory =
        load_directory(transaction, intent_id)?.ok_or(ChainPostCloseError::SchemaRejected)?;
    if directory.fold_outcome != "Available" {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let selections = load_selections(transaction, intent_id)?;
    if selections.len() != parent.clusters.len().min(20) {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    type Authority = (String, i64, i64, i64);
    let directory_authority: Authority = transaction
        .query_row(
            "SELECT lease_owner,lease_generation,materialized_at,run_version \
             FROM chain_post_close_board_directory_materials WHERE intent_id=?1",
            [intent_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| storage("position board parent"))?;
    let selection_authority = transaction
        .query_row(
            "SELECT lease_owner,lease_generation,selected_at,run_version \
             FROM chain_post_close_board_selections WHERE intent_id=?1 \
             ORDER BY run_version DESC LIMIT 1",
            [intent_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|_| storage("position board selection parent"))?;
    let authority = selection_authority.unwrap_or(directory_authority);
    Ok(PositionsParentFact {
        owner: authority.0,
        generation: u64::try_from(authority.1).map_err(|_| ChainPostCloseError::SchemaRejected)?,
        ready_at: authority.2,
        run_version: u64::try_from(authority.3).map_err(|_| ChainPostCloseError::SchemaRejected)?,
    })
}

fn validate_status_materials(
    connection: &Connection,
    intent_id: &IntentId,
    head: u64,
) -> Result<(), ChainPostCloseError> {
    type Row = (
        String,
        i64,
        i64,
        String,
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<Vec<u8>>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let (current_run_id, current_context, current_input, current_owner, current_generation, updated_at): (
        String,
        String,
        String,
        String,
        i64,
        i64,
    ) = connection
        .query_row(
            "SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,updated_at \
             FROM chain_post_close_runs WHERE intent_id=?1",
            [intent_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|_| storage("board status run read"))?;
    let current_generation =
        u64::try_from(current_generation).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut statement = connection
        .prepare(
            "SELECT kind,attempt_ordinal,result_run_version,result_sha256,request_sha256, \
                    provenance,projection_version,material_codec_version,material_bytes, \
                    material_length,material_sha256,run_id,run_context_sha256,input_sha256, \
                    lease_owner,lease_generation,prior_head_version,run_version,captured_at, \
                    legacy_layout_version \
             FROM chain_post_close_board_status_materials \
             WHERE intent_id=?1 ORDER BY kind,attempt_ordinal",
        )
        .map_err(|_| storage("board status materials read"))?;
    let rows = statement
        .query_map([intent_id.as_str()], |row| {
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
            ))
        })
        .map_err(|_| storage("board status materials read"))?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .map_err(|_| storage("board status materials read"))?;
    let status_result_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM chain_post_close_board_attempt_results \
             WHERE intent_id=?1 AND wire_outcome='Status'",
            [intent_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage("board status results read"))?;
    if usize::try_from(status_result_count).ok() != Some(rows.len()) {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    for row in rows {
        let kind = board_codec::parse_kind(&row.0)?;
        let attempt = u32::try_from(row.1).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let result_version =
            u64::try_from(row.2).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        struct ResultParent {
            run_version: i64,
            result_digest: String,
            request_digest: String,
            wire_outcome: String,
            run_id: String,
            context_digest: String,
            input_digest: String,
            owner: String,
            generation: i64,
            begin_run_version: i64,
            prior_head: i64,
            returned_at: i64,
            committed_at: i64,
            result_bytes: Vec<u8>,
            result_length: i64,
            begin_version: i64,
            begin_request_digest: String,
            begin_run_id: String,
            begin_context_digest: String,
            begin_input_digest: String,
            begin_owner: String,
            begin_generation: i64,
            begun_at: i64,
            result_codec: i64,
            continuation: String,
            retry_decision: String,
            backoff_ms: Option<i64>,
        }
        let parent: ResultParent = connection
            .query_row(
                "SELECT result.run_version,result.result_sha256,result.request_sha256, \
                        result.wire_outcome,result.run_id,result.run_context_sha256, \
                        result.input_sha256,result.lease_owner,result.lease_generation, \
                        result.begin_run_version,result.prior_head_version,result.returned_at, \
                        result.committed_at,result.result_bytes,result.result_length, \
                        begin.run_version,begin.request_sha256,begin.run_id, \
                        begin.run_context_sha256,begin.input_sha256,begin.lease_owner, \
                        begin.lease_generation,begin.begun_at,result.result_codec_version, \
                        result.continuation,result.retry_decision,result.backoff_ms \
                 FROM chain_post_close_board_attempt_results AS result \
                 JOIN chain_post_close_board_attempt_begins AS begin \
                   ON begin.intent_id=result.intent_id AND begin.kind=result.kind \
                  AND begin.attempt_ordinal=result.attempt_ordinal \
                 WHERE result.intent_id=?1 AND result.kind=?2 AND result.attempt_ordinal=?3",
                params![intent_id.as_str(), board_codec::kind_name(kind), attempt],
                |parent| {
                    Ok(ResultParent {
                        run_version: parent.get(0)?,
                        result_digest: parent.get(1)?,
                        request_digest: parent.get(2)?,
                        wire_outcome: parent.get(3)?,
                        run_id: parent.get(4)?,
                        context_digest: parent.get(5)?,
                        input_digest: parent.get(6)?,
                        owner: parent.get(7)?,
                        generation: parent.get(8)?,
                        begin_run_version: parent.get(9)?,
                        prior_head: parent.get(10)?,
                        returned_at: parent.get(11)?,
                        committed_at: parent.get(12)?,
                        result_bytes: parent.get(13)?,
                        result_length: parent.get(14)?,
                        begin_version: parent.get(15)?,
                        begin_request_digest: parent.get(16)?,
                        begin_run_id: parent.get(17)?,
                        begin_context_digest: parent.get(18)?,
                        begin_input_digest: parent.get(19)?,
                        begin_owner: parent.get(20)?,
                        begin_generation: parent.get(21)?,
                        begun_at: parent.get(22)?,
                        result_codec: parent.get(23)?,
                        continuation: parent.get(24)?,
                        retry_decision: parent.get(25)?,
                        backoff_ms: parent.get(26)?,
                    })
                },
            )
            .map_err(|_| storage("board status parent read"))?;
        let parent_generation =
            u64::try_from(parent.generation).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let parent_authority_valid = parent_generation < current_generation
            || (parent_generation == current_generation && parent.owner == current_owner);
        if u64::try_from(parent.run_version).ok() != Some(result_version)
            || parent.result_digest != row.3
            || parent.request_digest != row.4
            || parent.wire_outcome != "Status"
            || parent.result_length != i64::try_from(parent.result_bytes.len()).unwrap_or(-1)
            || parent.result_digest != raw_digest(&parent.result_bytes).as_str()
            || parent.begin_run_version != parent.begin_version
            || parent.prior_head != parent.begin_version
            || parent.run_version != parent.prior_head.checked_add(1).unwrap_or(-1)
            || parent.run_id != current_run_id
            || parent.context_digest != current_context
            || parent.input_digest != current_input
            || parent.request_digest != parent.begin_request_digest
            || parent.run_id != parent.begin_run_id
            || parent.context_digest != parent.begin_context_digest
            || parent.input_digest != parent.begin_input_digest
            || parent.owner != parent.begin_owner
            || parent.generation != parent.begin_generation
            || result_version > head
            || parent.returned_at < parent.begun_at
            || parent.committed_at < parent.returned_at
            || parent.committed_at > updated_at
            || !parent_authority_valid
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let decoded_result = board_codec::decode_result(&parent.result_bytes)?;
        if parent.result_codec != 1
            || parent.continuation != decoded_result.continuation()
            || parent.retry_decision != decoded_result.retry_decision()
            || parent
                .backoff_ms
                .and_then(|value| u64::try_from(value).ok())
                != decoded_result.backoff_ms()
            || match parent.continuation.as_str() {
                "Retry" => decoded_result.confirmed_retry_backoff()?.is_none(),
                "Terminal" => decoded_result.terminal_status()?.is_none(),
                _ => true,
            }
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        match row.5.as_str() {
            "LegacyV5Absent" => {
                if row.6.is_some()
                    || row.7.is_some()
                    || row.8.is_some()
                    || row.9.is_some()
                    || row.10.is_some()
                    || row.11.is_some()
                    || row.12.is_some()
                    || row.13.is_some()
                    || row.14.is_some()
                    || row.15.is_some()
                    || row.16.is_some()
                    || row.17.is_some()
                    || row.18.is_some()
                    || row.19 != Some(6)
                {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
            }
            "Captured" => {
                let (
                    Some(projection),
                    Some(codec),
                    Some(bytes),
                    Some(length),
                    Some(digest),
                    Some(run_id),
                    Some(context),
                    Some(input),
                    Some(owner),
                    Some(generation),
                    Some(prior),
                    Some(version),
                    Some(captured_at),
                    None,
                ) = (
                    row.6, row.7, row.8, row.9, row.10, row.11, row.12, row.13, row.14, row.15,
                    row.16, row.17, row.18, row.19,
                )
                else {
                    return Err(ChainPostCloseError::SchemaRejected);
                };
                let generation =
                    u64::try_from(generation).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let prior =
                    u64::try_from(prior).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let version =
                    u64::try_from(version).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let authority_valid = generation < current_generation
                    || (generation == current_generation && owner == current_owner);
                if projection != 1
                    || codec != 1
                    || length != i64::try_from(bytes.len()).unwrap_or(-1)
                    || digest != raw_digest(&bytes).as_str()
                    || run_id != current_run_id
                    || context != current_context
                    || input != current_input
                    || run_id != parent.run_id
                    || context != parent.context_digest
                    || input != parent.input_digest
                    || owner != parent.owner
                    || generation != parent_generation
                    || prior != result_version
                    || prior.checked_add(1) != Some(version)
                    || version > head
                    || captured_at < parent.committed_at
                    || captured_at > updated_at
                    || !authority_valid
                {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                board_error_codec::decode_status(&bytes)?;
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        }
    }
    Ok(())
}
