//! Same-connection chain post-close persistence owned by BusinessIntentStore.
#![cfg_attr(not(test), allow(dead_code))]

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use anyhow::Result as AnyResult;
use chrono::{Local, NaiveDate, TimeZone};
use futures::stream::{FuturesUnordered, StreamExt};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::data_gateway::grpc_source::ConnectedBoardQueries;
use crate::data_gateway::grpc_source::GrpcSource;
use crate::database::data_acquisition_audit::validate_acquisition_chain_in_transaction;
use crate::database::global_schema_catalog_v1::verify_br159_acquisition_catalog;
use crate::market_data::TopStock;
use crate::monitor::push_job::{
    derive_intent_id, derive_occurrence_id, raw_digest, AudienceId, BusinessDate,
    CompletionOwnerId, IntentId, IntentIdentityMaterial, LocalChainPostCloseConfig,
    LocalChainPostCloseContext, Namespace, OccurrenceFamily, OccurrenceIdentityMaterial,
    OccurrenceKey, RunId, Sha256Digest, SourceContractId, SubjectId, UtcMicros,
};
use crate::pipeline::chain_analysis::preparation::{
    build_cluster_material, parse_concept_provider_raw, ChainPreparationIo, ConceptEffectClock,
    ConceptProviderRawIo, DragonTigerObservationClock, FixedClusterConfiguration,
    MacroObservationClock, ModelEffect, ModelStage, ModelsObservationClock,
    PositionObservationClock, PreparationStop, PreparedChainAnalysis, SearchStage,
    UnmigratedStage,
};
use crate::push_foundation::LeaseOwnerId;
use crate::search_service::SearchService;

use super::BusinessIntentStore;

#[path = "chain_post_close_board.rs"]
mod board;
#[path = "chain_post_close_board_codec.rs"]
mod board_codec;
#[path = "chain_post_close_board_error_codec.rs"]
mod board_error_codec;
#[path = "chain_post_close_cluster.rs"]
mod cluster;
#[path = "chain_post_close_codec.rs"]
mod codec;
#[path = "chain_post_close_concept_rpc.rs"]
mod concept_rpc;
#[path = "chain_post_close_concept_rpc_codec.rs"]
mod concept_rpc_codec;
#[path = "chain_post_close_concept_rpc_driver.rs"]
mod concept_rpc_driver;
#[path = "chain_post_close_dragon_tiger.rs"]
mod dragon_tiger;
#[path = "chain_post_close_dragon_tiger_codec.rs"]
mod dragon_tiger_codec;
#[path = "chain_post_close_dragon_tiger_driver.rs"]
mod dragon_tiger_driver;
#[path = "chain_post_close_macro_codec.rs"]
mod macro_codec;
#[path = "chain_post_close_macro_driver.rs"]
mod macro_driver;
#[path = "chain_post_close_macro_driver_v11.rs"]
mod macro_driver_v11;
#[path = "chain_post_close_macro_live.rs"]
mod macro_live;
#[path = "chain_post_close_macro_native.rs"]
mod macro_native;
#[path = "chain_post_close_macro_plan_v3.rs"]
mod macro_plan_v3;
#[path = "chain_post_close_macro_recovery.rs"]
mod macro_recovery;
#[path = "chain_post_close_macro.rs"]
mod macro_stage;
#[path = "chain_post_close_models.rs"]
mod models;
#[path = "chain_post_close_position_concept_rpc.rs"]
mod position_concept_rpc;
#[path = "chain_post_close_positions.rs"]
mod positions;
#[path = "chain_post_close_positions_codec.rs"]
mod positions_codec;
#[path = "chain_post_close_schema.rs"]
mod schema;

#[cfg(test)]
#[path = "chain_post_close_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "chain_post_close_v2_tests.rs"]
mod v2_tests;

pub(crate) use board::BoardDirectoryRecovery;
pub(crate) use cluster::ClusterApplicationRecovery;
pub(crate) use codec::FixedChainPreparationInput;
pub(crate) use dragon_tiger::DragonTigerProjectionFailed;

const COMPLETION_OWNER: &str = "monitor_loop::CHAIN_POST_LAST[calendar_date]";
const SOURCE_CONTRACT: &str = "chain-post-close-passed-input-v1";
const PRODUCER_ID: &str = "chain-post-close-timer";
const UNIT_ID: &str = "MU-chain-post-close";
const TEMPLATE_VERSION: &str = "chain-analysis-prepared-v1";

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ChainPostCloseError {
    #[error("chain post-close Macro has not been planned")]
    MacroNotStarted,
    #[error("chain post-close production trust roots unavailable")]
    ProductionRefused,
    #[error("chain post-close schema is not installed")]
    NotInstalled,
    #[error("chain post-close bundled schema failed verification")]
    BundleRejected,
    #[error("chain post-close persisted schema failed verification")]
    SchemaRejected,
    #[error("chain post-close schema or artifact codec version is unsupported")]
    UnsupportedVersion,
    #[error("legacy Macro plan has no authority for full execution")]
    LegacyPlanExecutionUnsupported,
    #[error("Macro plan lacks original independent Local route evidence")]
    MissingPersistedLocalRoute,
    #[error("chain post-close SQLite connection safeguards are unavailable")]
    ConnectionSafeguardFailed,
    #[error("chain post-close storage operation failed: {operation}")]
    StorageFailed { operation: &'static str },
    #[error("invalid local chain configuration: {check}")]
    InvalidConfiguration { check: &'static str },
    #[error("invalid fixed chain input: {check}")]
    InvalidInput { check: &'static str },
    #[error("fixed occurrence conflicts with existing run")]
    RunConflict { intent_id: String },
    #[error("run lease is currently held")]
    LeaseHeld { intent_id: String },
    #[error("run lease capability is stale")]
    StaleLease { intent_id: String },
    #[error("run lease has expired")]
    LeaseExpired { intent_id: String },
    #[error("run is missing")]
    RunMissing,
    #[error("effect is incomplete")]
    IncompleteEffect { intent_id: String },
    #[error("dragon-tiger has not started")]
    DragonTigerNotStarted,
}

/// Installation facts only. This receipt grants no business execution authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChainPostCloseSchemaReceipt {
    ddl_sha256: Sha256Digest,
    schema_version: u32,
}

impl ChainPostCloseSchemaReceipt {
    pub(crate) fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub(crate) fn artifact_codec_version(&self) -> u32 {
        1
    }

    pub(crate) fn input_codec_version(&self) -> u32 {
        1
    }

    pub(crate) fn stage_codec_version(&self) -> u32 {
        1
    }

    pub(crate) fn ddl_sha256(&self) -> &Sha256Digest {
        &self.ddl_sha256
    }
}

/// The parent store owns the connection; no path or raw connection escapes.
pub(crate) struct ChainPostClose<'store> {
    store: &'store mut BusinessIntentStore,
}

impl BusinessIntentStore {
    pub(crate) fn chain_post_close(&mut self) -> Result<ChainPostClose<'_>, ChainPostCloseError> {
        Err(ChainPostCloseError::ProductionRefused)
    }

    pub(crate) fn single_user_local_chain_post_close(
        &mut self,
        config: &LocalChainPostCloseConfig,
    ) -> Result<LocalChainPostClose<'_>, ChainPostCloseError> {
        let bundled = crate::monitor::push_job::MachineCatalog::bundled().map_err(|_| {
            ChainPostCloseError::InvalidConfiguration {
                check: "bundled catalog",
            }
        })?;
        if bundled.catalog_sha256() != config.expected_catalog_sha256() {
            return Err(ChainPostCloseError::InvalidConfiguration {
                check: "catalog digest",
            });
        }
        schema::verify_current(&mut self.connection)?;
        Ok(LocalChainPostClose {
            store: self,
            config: config.clone(),
        })
    }
}

pub(crate) struct LocalChainPostClose<'store> {
    store: &'store mut BusinessIntentStore,
    config: LocalChainPostCloseConfig,
}

pub(crate) struct RunLease {
    intent_id: IntentId,
    run_id: RunId,
    owner: LeaseOwnerId,
    generation: u64,
    head: u64,
    until: UtcMicros,
    input: FixedChainPreparationInput,
}

impl RunLease {
    pub(crate) fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn head_version(&self) -> u64 {
        self.head
    }
}

pub(crate) struct RunLeaseRequest {
    owner: LeaseOwnerId,
    now: UtcMicros,
    until: UtcMicros,
    expected: Option<u64>,
}

impl RunLeaseRequest {
    pub(crate) fn try_new(
        owner: LeaseOwnerId,
        now: UtcMicros,
        until: UtcMicros,
        expected: Option<u64>,
    ) -> Result<Self, ChainPostCloseError> {
        if until <= now {
            return Err(ChainPostCloseError::InvalidInput {
                check: "lease interval",
            });
        }
        Ok(Self {
            owner,
            now,
            until,
            expected,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ConceptProviderRequest {
    ordinal: u64,
    code: String,
}

impl ConceptProviderRequest {
    pub(crate) fn try_new(ordinal: u64, code: String) -> Result<Self, ChainPostCloseError> {
        if code.trim().is_empty() || code.contains('\0') || code.len() > 512 {
            return Err(ChainPostCloseError::InvalidInput {
                check: "concept code",
            });
        }
        Ok(Self { ordinal, code })
    }
}

pub(crate) struct ConceptProviderCall {
    intent_id: IntentId,
    run_id: RunId,
    owner: LeaseOwnerId,
    generation: u64,
    begin_version: u64,
    request: ConceptProviderRequest,
}

pub(crate) enum ConceptProviderAdmission {
    Call(ConceptProviderCall),
    Replay(StoredConceptProviderResult),
}

pub(crate) enum ConceptProviderRawResult {
    Returned(String),
    BusinessError(String),
}

impl ConceptProviderRawResult {
    pub(crate) fn returned(value: String) -> Self {
        Self::Returned(value)
    }
}

#[derive(Clone)]
pub(crate) struct StoredConceptProviderResult {
    ordinal: u64,
    code: String,
    outcome: String,
    bytes: Vec<u8>,
    run_version: u64,
}

#[derive(Clone)]
pub(crate) struct StageBeginFact {
    ordinal: u64,
    code: String,
    owner: String,
    generation: u64,
    run_version: u64,
    begun_at: i64,
}

#[derive(Clone)]
pub(crate) struct StageResultFact {
    ordinal: u64,
    outcome: String,
    bytes: Vec<u8>,
    owner: String,
    generation: u64,
    run_version: u64,
    committed_at: i64,
}

pub(crate) struct RunRecovery {
    context: crate::monitor::push_job::RunContext,
    input: FixedChainPreparationInput,
    owner: String,
    generation: u64,
    head: u64,
    updated_at: i64,
    begins: Vec<StageBeginFact>,
    results: Vec<StageResultFact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConceptEffectRecoveryState {
    NeverStarted,
    BegunUnconfirmed,
    Confirmed,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ConceptEffectRecovery {
    ordinal: u64,
    code: String,
    state: ConceptEffectRecoveryState,
}

pub(crate) struct ConceptBatchInspection {
    concepts: HashMap<String, Vec<String>>,
    applied_codes: Vec<String>,
    effects: Vec<ConceptEffectRecovery>,
    complete: bool,
}

impl ConceptBatchInspection {
    pub(crate) fn concepts(&self) -> &HashMap<String, Vec<String>> {
        &self.concepts
    }

    pub(crate) fn applied_codes(&self) -> &[String] {
        &self.applied_codes
    }

    pub(crate) fn effects(&self) -> &[ConceptEffectRecovery] {
        &self.effects
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }
}

impl ConceptEffectRecovery {
    pub(crate) fn ordinal(&self) -> u64 {
        self.ordinal
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn state(&self) -> ConceptEffectRecoveryState {
        self.state
    }
}

struct StoredConceptCacheWrite {
    ordinal: u64,
    code: String,
    provider_result_run_version: u64,
    concepts: Vec<String>,
    owner: String,
    generation: u64,
    run_version: u64,
    written_at: i64,
}

impl RunRecovery {
    pub(crate) fn context(&self) -> &crate::monitor::push_job::RunContext {
        &self.context
    }

    pub(crate) fn fixed_input(&self) -> &FixedChainPreparationInput {
        &self.input
    }

    pub(crate) fn lease_generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn head_version(&self) -> u64 {
        self.head
    }

    pub(crate) fn begins(&self) -> &[StageBeginFact] {
        &self.begins
    }

    pub(crate) fn results(&self) -> &[StageResultFact] {
        &self.results
    }
}

fn storage(operation: &'static str) -> ChainPostCloseError {
    ChainPostCloseError::StorageFailed { operation }
}

fn identity(
    unit_id: crate::monitor::push_job::UnitId,
    occurrence: crate::monitor::push_job::OccurrenceId,
) -> Result<IntentId, ChainPostCloseError> {
    Ok(derive_intent_id(&IntentIdentityMaterial::new(
        Namespace::Production,
        unit_id,
        CompletionOwnerId::try_new(COMPLETION_OWNER.to_owned()).map_err(|_| {
            ChainPostCloseError::InvalidConfiguration {
                check: "completion owner",
            }
        })?,
        SourceContractId::try_new(SOURCE_CONTRACT.to_owned()).map_err(|_| {
            ChainPostCloseError::InvalidConfiguration {
                check: "source contract",
            }
        })?,
        occurrence,
        SubjectId::Global,
        AudienceId::try_new("single-user-local-owner".to_owned()).map_err(|_| {
            ChainPostCloseError::InvalidConfiguration {
                check: "local audience",
            }
        })?,
    )))
}

fn intent_for(context: &LocalChainPostCloseContext) -> Result<IntentId, ChainPostCloseError> {
    identity(
        context.run_context().unit_id().clone(),
        context.run_context().occurrence().clone(),
    )
}

impl<'store> LocalChainPostClose<'store> {
    pub(crate) fn acquire_run(
        &mut self,
        context: LocalChainPostCloseContext,
        mut input: FixedChainPreparationInput,
        request: RunLeaseRequest,
    ) -> Result<RunLease, ChainPostCloseError> {
        if !self.config.accepts(context.run_context())
            || context.run_context().namespace() != &Namespace::Production
            || context.run_context().unit_id().as_str() != UNIT_ID
            || context.run_context().business_date().as_str() != input.business_date()
            || context.occurrence_family().as_str()
                != "calendar date / 15:30≤t<15:35 / latest completed business date"
            || context.source_contract_id().as_str() != SOURCE_CONTRACT
        {
            return Err(ChainPostCloseError::InvalidConfiguration {
                check: "context binding",
            });
        }
        let intent_id = intent_for(&context)?;
        let caller_bytes = input.caller_bytes()?;
        let context_bytes = context.run_context().canonical_bytes();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout(&transaction)?;

        let existing = transaction
            .query_row(
                "SELECT run_id,context_bytes,input_bytes FROM chain_post_close_runs \
                 WHERE intent_id=?1",
                [intent_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| storage("run lookup"))?;
        if let Some((run_id, saved_context, saved_input)) = existing {
            let saved = inspect_run_on(&transaction, &intent_id)?;
            if run_id != context.run_context().run_id().as_str()
                || saved_context != context_bytes
                || saved_input != saved.input.encode()?
                || saved.input.caller_bytes()? != caller_bytes
            {
                return Err(ChainPostCloseError::RunConflict {
                    intent_id: intent_id.as_str().to_owned(),
                });
            }
            let lease = resume_on(&transaction, &intent_id, request)?;
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(lease);
        }

        let captured = context.run_context().captured_business_time().get();
        let local = chrono::Utc
            .timestamp_micros(captured)
            .single()
            .ok_or(ChainPostCloseError::InvalidInput {
                check: "captured time",
            })?
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("valid offset"));
        let cutoff = (local - chrono::Duration::days(7))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let cache_rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT code,concepts,updated_at FROM stock_concepts \
                     WHERE updated_at>=?1 ORDER BY code",
                )
                .map_err(|_| storage("cache prepare"))?;
            let rows = statement
                .query_map([&cutoff], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|_| storage("cache read"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| storage("cache read"))?;
            rows.into_iter()
                .map(|(code, concepts, updated_at)| {
                    codec::CacheRow::try_new(code, concepts, updated_at)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        input.attach_cache(cutoff, cache_rows)?;
        let input_bytes = input.encode()?;
        let input_digest = raw_digest(&input_bytes);
        let context_digest = context.run_context().canonical_sha256();
        if raw_digest(&context_bytes) != context_digest {
            return Err(ChainPostCloseError::InvalidConfiguration {
                check: "context digest",
            });
        }
        transaction
            .execute(
                "INSERT INTO chain_post_close_runs( \
                 intent_id,run_id,context_codec_version,context_bytes,context_length, \
                 run_context_sha256,namespace,unit_id,producer_id,phase,occurrence_id, \
                 occurrence_family,occurrence_key,calendar_date,business_date,completion_owner, \
                 source_contract_id,source_contract_version,template_version,layout_version, \
                 input_codec_version,input_bytes,input_length,input_sha256,lease_owner, \
                 lease_generation,lease_until,head_version,created_at,updated_at) \
                 VALUES(?1,?2,1,?3,?4,?5,'Production',?6,?7,'Postclose',?8,?9,?10, \
                 ?11,?12,?13,?14,'1',?15,2,1,?16,?17,?18,?19,1,?20,0,?21,?21)",
                params![
                    intent_id.as_str(),
                    context.run_context().run_id().as_str(),
                    &context_bytes,
                    i64::try_from(context_bytes.len()).map_err(|_| storage("context length"))?,
                    context_digest.as_str(),
                    UNIT_ID,
                    PRODUCER_ID,
                    context.run_context().occurrence().as_str(),
                    context.occurrence_family().as_str(),
                    context.occurrence_key().as_str(),
                    context.run_context().calendar_date().as_str(),
                    context.run_context().business_date().as_str(),
                    COMPLETION_OWNER,
                    SOURCE_CONTRACT,
                    TEMPLATE_VERSION,
                    &input_bytes,
                    i64::try_from(input_bytes.len()).map_err(|_| storage("input length"))?,
                    input_digest.as_str(),
                    request.owner.as_str(),
                    request.until.get(),
                    request.now.get(),
                ],
            )
            .map_err(|_| storage("insert run"))?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(RunLease {
            intent_id,
            run_id: context.run_context().run_id().clone(),
            owner: request.owner,
            generation: 1,
            head: 0,
            until: request.until,
            input,
        })
    }

    pub(crate) fn resume_run(
        &mut self,
        intent_id: &IntentId,
        request: RunLeaseRequest,
    ) -> Result<RunLease, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout(&transaction)?;
        let lease = resume_on(&transaction, intent_id, request)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(lease)
    }

    pub(crate) fn inspect_run(
        &mut self,
        intent_id: &IntentId,
    ) -> Result<RunRecovery, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, intent_id)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(recovery)
    }

    pub(crate) fn inspect_concept_batch(
        &mut self,
        intent_id: &IntentId,
    ) -> Result<ConceptBatchInspection, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout(&transaction)?;
        let inspection = inspect_concept_batch_on(&transaction, intent_id)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(inspection)
    }

    pub(crate) fn begin_concept_provider(
        &mut self,
        mut lease: RunLease,
        request: ConceptProviderRequest,
        now: UtcMicros,
    ) -> Result<(RunLease, ConceptProviderAdmission), ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let layout_version = schema::runtime_layout_version(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        if recovery.input.encode()? != lease.input.encode()?
            || recovery.context.run_id() != &lease.run_id
            || recovery.generation != lease.generation
            || recovery.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        check_lease(&transaction, &lease, now)?;
        match layout_version {
            2 => validate_first_missing_request(&lease.input, &request)?,
            3..=6 => validate_missing_request(&lease.input, &request)?,
            _ => return Err(ChainPostCloseError::UnsupportedVersion),
        }

        let existing_begin = load_begin(&transaction, &lease.intent_id, request.ordinal)?;
        let existing_result = load_result(&transaction, &lease.intent_id, request.ordinal)?;
        if let Some(begin) = existing_begin {
            if begin.code != request.code {
                return Err(ChainPostCloseError::InvalidInput {
                    check: "effect request mismatch",
                });
            }
            if let Some(result) = existing_result {
                transaction.commit().map_err(|_| storage("commit"))?;
                return Ok((lease, ConceptProviderAdmission::Replay(result)));
            }
            return Err(ChainPostCloseError::IncompleteEffect {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        if existing_result.is_some() {
            return Err(ChainPostCloseError::SchemaRejected);
        }

        let request_bytes = request.code.as_bytes();
        let previous = lease.head;
        lease.head = lease
            .head
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        let changed = transaction
            .execute(
                "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
                 WHERE intent_id=?3 AND lease_owner=?4 AND lease_generation=?5 \
                   AND head_version=?6 AND lease_until>?2",
                params![
                    lease.head,
                    now.get(),
                    lease.intent_id.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                ],
            )
            .map_err(|_| storage("begin cas"))?;
        if changed != 1 {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        transaction
            .execute(
                "INSERT INTO chain_post_close_stage_begins( \
                 intent_id,effect_kind,effect_ordinal,effect_key,request_codec_version, \
                 request_bytes,request_length,request_sha256,lease_owner,lease_generation, \
                 run_version,begun_at) \
                 VALUES(?1,'ConceptProvider',?2,?3,1,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    lease.intent_id.as_str(),
                    request.ordinal,
                    request.code,
                    request_bytes,
                    i64::try_from(request_bytes.len()).map_err(|_| storage("request length"))?,
                    raw_digest(request_bytes).as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("begin insert"))?;
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        let call = ConceptProviderCall {
            intent_id: lease.intent_id.clone(),
            run_id: lease.run_id.clone(),
            owner: lease.owner.clone(),
            generation: lease.generation,
            begin_version: lease.head,
            request,
        };
        Ok((lease, ConceptProviderAdmission::Call(call)))
    }

    pub(crate) fn record_concept_provider_result(
        &mut self,
        mut lease: RunLease,
        call: ConceptProviderCall,
        result: ConceptProviderRawResult,
        now: UtcMicros,
    ) -> Result<(RunLease, StoredConceptProviderResult), ChainPostCloseError> {
        if call.intent_id != lease.intent_id
            || call.run_id != lease.run_id
            || call.owner != lease.owner
            || call.generation != lease.generation
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        if recovery.input.encode()? != lease.input.encode()?
            || recovery.context.run_id() != &lease.run_id
            || recovery.generation != lease.generation
            || recovery.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        check_lease(&transaction, &lease, now)?;
        let begin = load_begin(&transaction, &lease.intent_id, call.request.ordinal)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if begin.code != call.request.code
            || begin.run_version != call.begin_version
            || begin.owner != call.owner.as_str()
            || begin.generation != call.generation
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        if load_result(&transaction, &lease.intent_id, call.request.ordinal)?.is_some() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let (outcome, text) = match result {
            ConceptProviderRawResult::Returned(text) => ("Returned", text),
            ConceptProviderRawResult::BusinessError(text) => ("BusinessError", text),
        };
        let bytes = text.into_bytes();
        let digest = raw_digest(&bytes);
        let previous = lease.head;
        lease.head = lease
            .head
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        let changed = transaction
            .execute(
                "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
                 WHERE intent_id=?3 AND lease_owner=?4 AND lease_generation=?5 \
                   AND head_version=?6 AND lease_until>?2",
                params![
                    lease.head,
                    now.get(),
                    lease.intent_id.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                ],
            )
            .map_err(|_| storage("result cas"))?;
        if changed != 1 {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        transaction
            .execute(
                "INSERT INTO chain_post_close_stage_results( \
                 intent_id,effect_kind,effect_ordinal,outcome,result_codec_version, \
                 result_bytes,result_length,result_sha256,lease_owner,lease_generation, \
                 run_version,returned_at,committed_at) \
                 VALUES(?1,'ConceptProvider',?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?10)",
                params![
                    lease.intent_id.as_str(),
                    call.request.ordinal,
                    outcome,
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("result length"))?,
                    digest.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("result insert"))?;
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        let result_run_version = lease.head;
        Ok((
            lease,
            StoredConceptProviderResult {
                ordinal: call.request.ordinal,
                code: call.request.code,
                outcome: outcome.to_owned(),
                bytes,
                run_version: result_run_version,
            },
        ))
    }

    fn commit_concept_cache_write(
        &mut self,
        mut lease: RunLease,
        result: &StoredConceptProviderResult,
        now: UtcMicros,
    ) -> Result<(RunLease, Vec<String>), ChainPostCloseError> {
        let raw =
            std::str::from_utf8(&result.bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let concepts = parse_concept_provider_raw(raw, &result.code).map_err(|_| {
            ChainPostCloseError::InvalidInput {
                check: "concept provider result",
            }
        })?;
        let concepts_bytes = serde_json::to_vec(&concepts).map_err(|_| storage("cache codec"))?;
        let cache_updated_at = chrono::Utc
            .timestamp_micros(now.get())
            .single()
            .ok_or(ChainPostCloseError::InvalidInput {
                check: "cache timestamp",
            })?
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        schema::verify_runtime_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        if recovery.input.encode()? != lease.input.encode()?
            || recovery.context.run_id() != &lease.run_id
            || recovery.generation != lease.generation
            || recovery.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        check_lease(&transaction, &lease, now)?;
        let persisted = load_result(&transaction, &lease.intent_id, result.ordinal)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if persisted.code != result.code
            || persisted.outcome != "Returned"
            || persisted.bytes != result.bytes
            || persisted.run_version != result.run_version
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        if let Some(applied) = load_cache_write(&transaction, &lease.intent_id, result.ordinal)? {
            if applied.code != result.code
                || applied.provider_result_run_version != result.run_version
                || applied.concepts != concepts
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok((lease, concepts));
        }

        let previous = lease.head;
        lease.head = lease
            .head
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        let changed = transaction
            .execute(
                "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
                 WHERE intent_id=?3 AND lease_owner=?4 AND lease_generation=?5 \
                   AND head_version=?6 AND lease_until>?2",
                params![
                    lease.head,
                    now.get(),
                    lease.intent_id.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                ],
            )
            .map_err(|_| storage("cache cas"))?;
        if changed != 1 {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        let concepts_text = std::str::from_utf8(&concepts_bytes)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO stock_concepts(code,concepts,updated_at) \
                 VALUES(?1,?2,?3)",
                params![result.code.as_str(), concepts_text, &cache_updated_at],
            )
            .map_err(|_| storage("cache write"))?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_concept_cache_writes( \
                 intent_id,effect_kind,effect_ordinal,code,provider_result_run_version, \
                 concepts_codec_version,concepts_bytes,concepts_length,concepts_sha256, \
                 cache_updated_at,lease_owner,lease_generation,run_version,written_at) \
                 VALUES(?1,'ConceptProvider',?2,?3,?4,1,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    lease.intent_id.as_str(),
                    result.ordinal,
                    result.code.as_str(),
                    result.run_version,
                    &concepts_bytes,
                    i64::try_from(concepts_bytes.len()).map_err(|_| storage("cache length"))?,
                    raw_digest(&concepts_bytes).as_str(),
                    &cache_updated_at,
                    lease.owner.as_str(),
                    lease.generation,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("cache fact"))?;
        validate_run_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        if transaction.commit().is_err() {
            if !self.store.connection.is_autocommit() {
                let _ = self.store.connection.execute_batch("ROLLBACK;");
            }
            return Err(storage("commit"));
        }
        Ok((lease, concepts))
    }

    pub(crate) fn first_concept_preparation_io<'local, 'provider, P, C>(
        &'local mut self,
        lease: RunLease,
        provider: &'provider mut P,
        clock: &'provider C,
    ) -> Result<LocalConceptPreparationIo<'local, 'provider, 'store, P, C>, ChainPostCloseError>
    where
        P: ConceptProviderRawIo,
        C: ConceptEffectClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 2)?;
        check_lease(&self.store.connection, &lease, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptPreparationIo {
            local: self,
            lease: Some(lease),
            provider,
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
        })
    }

    pub(crate) fn concept_batch_preparation_io<'local, 'provider, P, C>(
        &'local mut self,
        lease: RunLease,
        provider: &'provider P,
        clock: &'provider C,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        P: ConceptProviderRawIo,
        C: ConceptEffectClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 3)?;
        check_lease(&self.store.connection, &lease, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Raw(provider),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: None,
            board_source: None,
            position_observation_clock: None,
            position_rpc: false,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn cluster_preparation_io<'local, 'provider, P, C>(
        &'local mut self,
        lease: RunLease,
        provider: &'provider P,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        P: ConceptProviderRawIo,
        C: ConceptEffectClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 4)?;
        check_lease(&self.store.connection, &lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Raw(provider),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: None,
            position_observation_clock: None,
            position_rpc: false,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn board_preparation_io<'local, 'provider, P, C>(
        &'local mut self,
        lease: RunLease,
        provider: &'provider P,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        board_source: &'provider GrpcSource,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        P: ConceptProviderRawIo,
        C: ConceptEffectClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 5)?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Raw(provider),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Legacy(board_source)),
            position_observation_clock: None,
            position_rpc: false,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn board_preparation_io_v6<'local, 'provider, P, C>(
        &'local mut self,
        lease: RunLease,
        provider: &'provider P,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        board_source: &'provider GrpcSource,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        P: ConceptProviderRawIo,
        C: ConceptEffectClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 6)?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Raw(provider),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Legacy(board_source)),
            position_observation_clock: None,
            position_rpc: false,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn concept_rpc_preparation_io_v7<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: ConceptEffectClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 7)?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Rpc(queries),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Connected(queries)),
            position_observation_clock: None,
            position_rpc: false,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn position_concept_rpc_preparation_io_v9<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: PositionObservationClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 9)?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Rpc(queries),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Connected(queries)),
            position_observation_clock: Some(clock),
            position_rpc: true,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn positions_preparation_io_v8<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: PositionObservationClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 8)?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Rpc(queries),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Connected(queries)),
            position_observation_clock: Some(clock),
            position_rpc: false,
            dragon_tiger_source: None,
            dragon_tiger_clock: None,
            macro_input: None,
            models_input: None,
        })
    }

    pub(crate) fn macro_preparation_io_v11<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        parent_source: &'provider GrpcSource,
        source: &'provider GrpcSource,
        search_service: &'provider SearchService,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: MacroObservationClock,
    {
        self.macro_preparation_io_at_layout(
            lease,
            queries,
            clock,
            configuration,
            parent_source,
            source,
            search_service,
            MacroLayout::V11,
        )
    }

    pub(crate) fn macro_preparation_io_v12<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        parent_source: &'provider GrpcSource,
        source: &'provider GrpcSource,
        search_service: &'provider SearchService,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: MacroObservationClock,
    {
        self.macro_preparation_io_at_layout(
            lease,
            queries,
            clock,
            configuration,
            parent_source,
            source,
            search_service,
            MacroLayout::V12,
        )
    }

    fn macro_preparation_io_at_layout<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        parent_source: &'provider GrpcSource,
        source: &'provider GrpcSource,
        search_service: &'provider SearchService,
        layout: MacroLayout,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: MacroObservationClock,
    {
        schema::verify_runtime_layout_version(
            &self.store.connection,
            match layout {
                MacroLayout::V11 => 11,
                MacroLayout::V12 => 12,
            },
        )?;
        self.recover_macro_parent(&lease, clock.now())?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Rpc(queries),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Connected(queries)),
            position_observation_clock: Some(clock),
            position_rpc: true,
            dragon_tiger_source: Some(parent_source),
            dragon_tiger_clock: Some(clock),
            macro_input: Some(MacroInput {
                source,
                clock,
                search_service,
                layout,
            }),
            models_input: None,
        })
    }

    /// V13: the full post-close chain, Models/Search/Report journaled in the
    /// same store. Macro facts keep the v12 shape.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn models_preparation_io_v13<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        parent_source: &'provider GrpcSource,
        source: &'provider GrpcSource,
        search_service: &'provider SearchService,
        analyzer: &'provider crate::analyzer::GeminiAnalyzer,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: ModelsObservationClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 13)?;
        self.recover_macro_parent(&lease, clock.now())?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Rpc(queries),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Connected(queries)),
            position_observation_clock: Some(clock),
            position_rpc: true,
            dragon_tiger_source: Some(parent_source),
            dragon_tiger_clock: Some(clock),
            macro_input: Some(MacroInput {
                source,
                clock,
                search_service,
                layout: MacroLayout::V12,
            }),
            models_input: Some(ModelsInput {
                analyzer,
                search_service,
                clock,
                session: None,
            }),
        })
    }

    pub(crate) fn dragon_tiger_preparation_io_v10<'local, 'provider, C>(
        &'local mut self,
        lease: RunLease,
        queries: &'provider ConnectedBoardQueries,
        clock: &'provider C,
        configuration: FixedClusterConfiguration,
        source: &'provider GrpcSource,
    ) -> Result<LocalConceptBatchPreparationIo<'local, 'provider, 'store, C>, ChainPostCloseError>
    where
        C: DragonTigerObservationClock,
    {
        schema::verify_runtime_layout_version(&self.store.connection, 10)?;
        self.admit_board(&lease, clock.now())?;
        let lease = self.fix_cluster_configuration(lease, configuration, clock.now())?;
        let intent_id = lease.intent_id.as_str().to_owned();
        Ok(LocalConceptBatchPreparationIo {
            local: self,
            lease: Some(lease),
            concept_source: ConceptSource::Rpc(queries),
            clock,
            cancelled: Rc::new(Cell::new(false)),
            intent_id,
            configuration: Some(configuration),
            board_source: Some(BoardSource::Connected(queries)),
            position_observation_clock: Some(clock),
            position_rpc: true,
            dragon_tiger_source: Some(source),
            dragon_tiger_clock: Some(clock),
            macro_input: None,
            models_input: None,
        })
    }
}

fn resume_on(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    request: RunLeaseRequest,
) -> Result<RunLease, ChainPostCloseError> {
    let recovery = inspect_run_on(connection, intent_id)?;
    let (lease_generation, lease_until): (i64, i64) = connection
        .query_row(
            "SELECT lease_generation,lease_until FROM chain_post_close_runs WHERE intent_id=?1",
            [intent_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage("run lease"))?;
    if request
        .expected
        .is_some_and(|expected| expected != recovery.head)
    {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: intent_id.as_str().to_owned(),
        });
    }
    if request.now.get() < lease_until {
        return Err(ChainPostCloseError::LeaseHeld {
            intent_id: intent_id.as_str().to_owned(),
        });
    }
    if request.expected.is_none() {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: intent_id.as_str().to_owned(),
        });
    }
    let next_generation = u64::try_from(lease_generation)
        .ok()
        .and_then(|generation| generation.checked_add(1))
        .ok_or_else(|| storage("lease generation"))?;
    let next_head = recovery
        .head
        .checked_add(1)
        .ok_or_else(|| storage("head overflow"))?;
    let changed = connection
        .execute(
            "UPDATE chain_post_close_runs SET lease_owner=?1,lease_generation=?2, \
             lease_until=?3,head_version=?4,updated_at=?5 \
             WHERE intent_id=?6 AND lease_generation=?7 AND head_version=?8 AND lease_until<=?5",
            params![
                request.owner.as_str(),
                next_generation,
                request.until.get(),
                next_head,
                request.now.get(),
                intent_id.as_str(),
                lease_generation,
                recovery.head,
            ],
        )
        .map_err(|_| storage("lease cas"))?;
    if changed != 1 {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: intent_id.as_str().to_owned(),
        });
    }
    Ok(RunLease {
        intent_id: intent_id.clone(),
        run_id: recovery.context.run_id().clone(),
        owner: request.owner,
        generation: next_generation,
        head: next_head,
        until: request.until,
        input: recovery.input,
    })
}

fn inspect_run_on(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
) -> Result<RunRecovery, ChainPostCloseError> {
    inspect_run_and_macro_on(connection, intent_id).map(|(run, _)| run)
}

fn inspect_run_and_macro_on(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
) -> Result<(RunRecovery, Option<macro_stage::MacroRecovery>), ChainPostCloseError> {
    inspect_run_and_macro_scoped(connection, intent_id, |_, _, _| Ok(()))
        .map(|(run, macro_recovery, _)| (run, macro_recovery))
}

fn inspect_run_and_macro_scoped<'transaction, 'connection, T>(
    connection: &'transaction Transaction<'connection>,
    intent_id: &IntentId,
    consume: impl for<'run> FnOnce(
        &'run RunRecovery,
        &Option<macro_stage::MacroRecovery>,
        dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>,
    ) -> Result<T, ChainPostCloseError>,
) -> Result<(RunRecovery, Option<macro_stage::MacroRecovery>, Option<T>), ChainPostCloseError> {
    let (layout_version, proof) = schema::transaction_layout(connection)?;
    inspect_run_and_macro_at_layout_with_catalog(
        connection,
        intent_id,
        layout_version,
        proof.as_ref(),
        consume,
    )
}

fn inspect_run_and_macro_with_catalog(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    proof: &schema::V12CatalogProof<'_, '_>,
) -> Result<(RunRecovery, Option<macro_stage::MacroRecovery>), ChainPostCloseError> {
    inspect_run_and_macro_scoped_with_catalog(connection, intent_id, proof, |_, _, _| Ok(()))
        .map(|(run, recovery, _)| (run, recovery))
}

fn inspect_run_and_macro_scoped_with_catalog<'transaction, 'connection, T>(
    connection: &'transaction Transaction<'connection>,
    intent_id: &IntentId,
    proof: &schema::V12CatalogProof<'_, '_>,
    consume: impl for<'run> FnOnce(
        &'run RunRecovery,
        &Option<macro_stage::MacroRecovery>,
        dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>,
    ) -> Result<T, ChainPostCloseError>,
) -> Result<(RunRecovery, Option<macro_stage::MacroRecovery>, Option<T>), ChainPostCloseError> {
    inspect_run_and_macro_at_layout_with_catalog(connection, intent_id, proof.layout(), Some(proof), consume)
}

fn inspect_run_and_macro_at_layout_with_catalog<'transaction, 'connection, T>(
    connection: &'transaction Transaction<'connection>,
    intent_id: &IntentId,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
    consume: impl for<'run> FnOnce(
        &'run RunRecovery,
        &Option<macro_stage::MacroRecovery>,
        dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>,
    ) -> Result<T, ChainPostCloseError>,
) -> Result<(RunRecovery, Option<macro_stage::MacroRecovery>, Option<T>), ChainPostCloseError> {
    if let Some(proof) = proof {
        if layout_version < 12 {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        schema::verify_v12_read_pass(connection, proof)?;
    }
    let recovery = inspect_run_on_with_catalog(connection, intent_id, layout_version, proof)?;
    if matches!(layout_version, 7 | 8 | 9 | 10 | 11 | 12 | 13) {
        concept_rpc::validate_concept_rpc_facts(connection, intent_id)?;
    }
    let validated_positions = if matches!(layout_version, 8 | 9 | 10 | 11 | 12 | 13) {
        let concepts = inspect_concept_batch_from_recovery_on(connection, intent_id, &recovery)?;
        let parent = cluster::validate_existing_cluster_facts_scoped(
            connection,
            intent_id,
            &recovery,
            &concepts,
            layout_version,
            proof,
        )?;
        if let Some(proof) = proof {
            let board = board::validate_and_capture_board_facts_for_read_pass(
                connection,
                intent_id,
                &recovery,
                parent.as_ref(),
                proof,
            )?;
            Some(positions::validate_position_facts_after_board_validation(
                connection,
                intent_id,
                &recovery,
                parent.as_ref(),
                proof,
                &board,
            )?)
        } else {
            board::validate_existing_board_facts_scoped(
                connection,
                intent_id,
                &recovery,
                parent.as_ref(),
                layout_version,
                None,
            )?;
            Some(
                positions::validate_existing_position_facts_and_capture_scoped(
                    connection,
                    intent_id,
                    &recovery,
                    parent.as_ref(),
                    layout_version,
                    None,
                )?,
            )
        }
    } else {
        None
    };
    if matches!(layout_version, 9 | 10 | 11 | 12 | 13) {
        if let Some(proof) = proof {
            position_concept_rpc::validate_facts_after_position_validation(
                connection,
                intent_id,
                &recovery,
                proof,
                validated_positions
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            )?;
        } else {
            position_concept_rpc::validate_facts_scoped(
                connection,
                intent_id,
                &recovery,
                layout_version,
                None,
            )?;
        }
    }
    let dragon_tiger = if matches!(layout_version, 10 | 11 | 12 | 13) {
        Some(
            dragon_tiger::validate_facts_and_capture_final_after_position_validation_scoped(
                connection,
                intent_id,
                &recovery,
                layout_version,
                validated_positions.ok_or(ChainPostCloseError::SchemaRejected)?,
                proof,
            )?,
        )
    } else {
        None
    };
    let (macro_recovery, consumed) = if matches!(layout_version, 11 | 12 | 13) {
        let dragon_tiger = dragon_tiger.ok_or(ChainPostCloseError::SchemaRejected)?;
        let macro_recovery = macro_stage::load_on_after_dragon_validation_scoped(
            connection,
            intent_id,
            &recovery,
            &dragon_tiger,
            proof,
        )?;
        let consumed = consume(&recovery, &macro_recovery, dragon_tiger)?;
        (macro_recovery, Some(consumed))
    } else {
        (None, None)
    };
    Ok((recovery, macro_recovery, consumed))
}

fn inspect_run_on_at_layout(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    layout_version: i64,
) -> Result<RunRecovery, ChainPostCloseError> {
    inspect_run_on_with_catalog(connection, intent_id, layout_version, None)
}

fn inspect_run_on_with_catalog(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<RunRecovery, ChainPostCloseError> {
    type RunRow = (
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
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
    );
    let row: RunRow = connection
        .query_row(
            "SELECT run_id,context_codec_version,context_bytes,context_length, \
             run_context_sha256,namespace,unit_id,producer_id,phase,occurrence_id, \
             occurrence_family,occurrence_key,calendar_date,business_date,completion_owner, \
             source_contract_id,input_codec_version,input_bytes,input_length,input_sha256, \
             lease_owner,lease_generation,head_version,updated_at \
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
                    row.get(22)?,
                    row.get(23)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("run read"))?
        .ok_or(ChainPostCloseError::RunMissing)?;
    let context = crate::monitor::push_job::RunContext::from_canonical_bytes(&row.2)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let input = FixedChainPreparationInput::decode(&row.17)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let context_length =
        i64::try_from(row.2.len()).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let input_length =
        i64::try_from(row.17.len()).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let derived_intent = identity(context.unit_id().clone(), context.occurrence().clone())?;
    let occurrence_material = OccurrenceIdentityMaterial::new(
        BusinessDate::parse(&row.13).map_err(|_| ChainPostCloseError::SchemaRejected)?,
        OccurrenceFamily::try_new(row.10.clone())
            .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        OccurrenceKey::try_new(row.11.clone()).map_err(|_| ChainPostCloseError::SchemaRejected)?,
    );
    if row.1 != 1
        || row.3 != context_length
        || row.4 != raw_digest(&row.2).as_str()
        || context.canonical_sha256().as_str() != row.4
        || row.0 != context.run_id().as_str()
        || row.5 != "Production"
        || row.6 != UNIT_ID
        || row.7 != PRODUCER_ID
        || row.8 != "Postclose"
        || row.9 != context.occurrence().as_str()
        || row.10 != "calendar date / 15:30≤t<15:35 / latest completed business date"
        || derive_occurrence_id(&occurrence_material) != *context.occurrence()
        || row.12 != context.calendar_date().as_str()
        || row.13 != context.business_date().as_str()
        || row.14 != COMPLETION_OWNER
        || row.15 != SOURCE_CONTRACT
        || row.16 != 1
        || row.18 != input_length
        || row.19 != raw_digest(&row.17).as_str()
        || input.business_date() != context.business_date().as_str()
        || derived_intent != *intent_id
        || row.20.trim().is_empty()
        || row.21 < 1
        || row.22 < 0
        || row.23 < 0
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let begins = read_begins(connection, intent_id)?;
    let results = read_results(connection, intent_id, &begins)?;
    let generation = u64::try_from(row.21).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let head = u64::try_from(row.22).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut versions = HashSet::new();
    if begins.iter().any(|fact| {
        fact.generation > generation
            || fact.run_version > head
            || fact.begun_at > row.23
            || (fact.generation == generation && fact.owner != row.20)
            || !versions.insert(fact.run_version)
    }) || results.iter().any(|fact| {
        fact.generation > generation
            || fact.run_version > head
            || fact.committed_at > row.23
            || (fact.generation == generation && fact.owner != row.20)
            || !versions.insert(fact.run_version)
    }) {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    if let Some(proof) = proof {
        if layout_version < 12 {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        proof.check(connection)?;
        validate_run_fact_versions_body(connection, intent_id, head, layout_version)?;
    } else {
        validate_run_fact_versions_at_layout(connection, intent_id, head, layout_version)?;
    }
    Ok(RunRecovery {
        context,
        input,
        owner: row.20,
        generation,
        head,
        updated_at: row.23,
        begins,
        results,
    })
}

pub(super) fn validate_run_fact_versions(
    connection: &Connection,
    intent_id: &IntentId,
    head: u64,
) -> Result<(), ChainPostCloseError> {
    let layout_version = schema::runtime_layout_version(connection)?;
    validate_run_fact_versions_at_layout(connection, intent_id, head, layout_version)
}

pub(super) fn validate_run_fact_versions_at_layout(
    connection: &Connection,
    intent_id: &IntentId,
    head: u64,
    layout_version: i64,
) -> Result<(), ChainPostCloseError> {
    if layout_version >= 12 {
        schema::verify_runtime_layout_version(connection, layout_version)?;
    }
    validate_run_fact_versions_body(connection, intent_id, head, layout_version)
}

fn validate_run_fact_versions_body(
    connection: &Connection,
    intent_id: &IntentId,
    head: u64,
    layout_version: i64,
) -> Result<(), ChainPostCloseError> {
    if matches!(layout_version, 10 | 11 | 12 | 13) {
        let tables = [
            "chain_post_close_stage_begins",
            "chain_post_close_stage_results",
            "chain_post_close_concept_cache_writes",
            "chain_post_close_cluster_configurations",
            "chain_post_close_cluster_materials",
            "chain_post_close_chain_daily_applications",
            "chain_post_close_board_attempt_begins",
            "chain_post_close_board_attempt_results",
            "chain_post_close_board_kind_finals",
            "chain_post_close_board_directory_materials",
            "chain_post_close_board_selections",
            "chain_post_close_board_status_materials",
            "chain_post_close_board_error_materials",
            "chain_post_close_concept_rpc_occurrences",
            "chain_post_close_concept_rpc_attempt_begins",
            "chain_post_close_concept_rpc_attempt_results",
            "chain_post_close_concept_rpc_status_materials",
            "chain_post_close_concept_rpc_error_materials",
            "chain_post_close_concept_rpc_finals",
            "chain_post_close_position_materials",
            "chain_post_close_position_concept_materials",
            "chain_post_close_position_concept_rpc_occurrences",
            "chain_post_close_position_concept_rpc_attempt_begins",
            "chain_post_close_position_concept_rpc_attempt_results",
            "chain_post_close_position_concept_rpc_status_materials",
            "chain_post_close_position_concept_rpc_error_materials",
            "chain_post_close_position_concept_rpc_finals",
            "chain_post_close_position_concept_cache_writes",
            "chain_post_close_dragon_tiger_occurrences",
            "chain_post_close_dragon_tiger_attempt_begins",
            "chain_post_close_dragon_tiger_attempt_results",
            "chain_post_close_dragon_tiger_status_materials",
            "chain_post_close_dragon_tiger_error_materials",
            "chain_post_close_dragon_tiger_finals",
        ];
        let mut unique = HashSet::new();
        let macro_tables = if matches!(layout_version, 11 | 12 | 13) {
            macro_stage::TABLES.as_slice()
        } else {
            &[]
        };
        let full_tables = if layout_version >= 12 {
            macro_recovery::TABLES.as_slice()
        } else {
            &[]
        };
        let models_tables = if layout_version >= 13 {
            models::TABLES.as_slice()
        } else {
            &[]
        };
        for table in tables
            .into_iter()
            .chain(macro_tables.iter().copied())
            .chain(full_tables.iter().copied())
            .chain(models_tables.iter().copied())
        {
            let sql = format!(
                "SELECT run_version FROM {table} WHERE intent_id=?1 AND run_version IS NOT NULL"
            );
            let mut statement = connection
                .prepare(&sql)
                .map_err(|_| storage("run fact versions"))?;
            let versions = statement
                .query_map([intent_id.as_str()], |row| row.get::<_, i64>(0))
                .map_err(|_| storage("run fact versions"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| storage("run fact versions"))?;
            if versions.into_iter().any(|version| {
                version < 1
                    || u64::try_from(version)
                        .ok()
                        .is_none_or(|value| value > head || !unique.insert(value))
            }) {
                return Err(ChainPostCloseError::SchemaRejected);
            }
        }
        return Ok(());
    }
    let sql = match layout_version {
        2 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1"
        }
        3 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1"
        }
        4 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=?1"
        }
        5 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=?1"
        }
        6 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_status_materials \
               WHERE intent_id=?1 AND run_version IS NOT NULL UNION ALL \
             SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=?1"
        }
        7 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_status_materials \
               WHERE intent_id=?1 AND run_version IS NOT NULL UNION ALL \
             SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_status_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_finals WHERE intent_id=?1"
        }
        8 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_status_materials \
               WHERE intent_id=?1 AND run_version IS NOT NULL UNION ALL \
             SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_status_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_materials WHERE intent_id=?1"
        }
        9 => {
            "SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_board_status_materials \
               WHERE intent_id=?1 AND run_version IS NOT NULL UNION ALL \
             SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_status_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_concept_rpc_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_rpc_occurrences WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_rpc_attempt_begins WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_rpc_attempt_results WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_rpc_status_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_rpc_error_materials WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_rpc_finals WHERE intent_id=?1 UNION ALL \
             SELECT run_version FROM chain_post_close_position_concept_cache_writes WHERE intent_id=?1"
        }
        _ => return Err(ChainPostCloseError::UnsupportedVersion),
    };
    let mut statement = connection
        .prepare(sql)
        .map_err(|_| storage("run fact versions"))?;
    let versions = statement
        .query_map([intent_id.as_str()], |row| row.get::<_, i64>(0))
        .map_err(|_| storage("run fact versions"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("run fact versions"))?;
    let mut unique = HashSet::new();
    if versions.into_iter().any(|version| {
        version < 1
            || u64::try_from(version)
                .ok()
                .is_none_or(|value| value > head || !unique.insert(value))
    }) {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn validate_owned_foreign_keys(connection: &Connection) -> Result<(), ChainPostCloseError> {
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| storage("chain fact foreign keys"))?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| storage("chain fact foreign keys"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("chain fact foreign keys"))?;
    if tables
        .iter()
        .any(|table| table.starts_with("chain_post_close_"))
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

pub(super) fn validate_all_runs_at_layout(
    transaction: &Transaction<'_>,
    layout_version: i64,
) -> Result<(), ChainPostCloseError> {
    if layout_version >= 12 {
        schema::verify_parent_layout_v12(transaction)?;
    }
    if !matches!(layout_version, 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    validate_owned_foreign_keys(transaction)?;

    let mut statement = transaction
        .prepare("SELECT intent_id FROM chain_post_close_runs ORDER BY intent_id")
        .map_err(|_| storage("migration run identities"))?;
    let stored = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| storage("migration run identities"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("migration run identities"))?;
    drop(statement);

    for stored_intent in stored {
        let digest = Sha256Digest::parse("chain post-close intent", &stored_intent)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let intent = IntentId::from_digest(&digest);
        if intent.as_str() != stored_intent {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let recovery = inspect_run_on_at_layout(transaction, &intent, layout_version)?;
        let concepts = inspect_concept_batch_from_recovery_on(transaction, &intent, &recovery)?;
        let parent = cluster::validate_existing_cluster_facts_at_layout(
            transaction,
            &intent,
            &recovery,
            &concepts,
            layout_version,
        )?;
        board::validate_existing_board_facts_at_layout(
            transaction,
            &intent,
            &recovery,
            parent.as_ref(),
            layout_version,
        )?;
        if matches!(layout_version, 7 | 8 | 9 | 10 | 11 | 12 | 13) {
            concept_rpc::validate_concept_rpc_fact_rows(transaction, &intent)?;
        }
        if matches!(layout_version, 8 | 9 | 10 | 11 | 12 | 13) {
            positions::validate_existing_position_facts_at_layout(
                transaction,
                &intent,
                &recovery,
                parent.as_ref(),
                layout_version,
            )?;
        }
        if matches!(layout_version, 9 | 10 | 11 | 12 | 13) {
            position_concept_rpc::validate_facts(transaction, &intent, &recovery, layout_version)?;
        }
        if matches!(layout_version, 10 | 11 | 12 | 13) {
            dragon_tiger::validate_facts_at_layout(
                transaction,
                &intent,
                &recovery,
                layout_version,
            )?;
        }
        if matches!(layout_version, 11 | 12 | 13) {
            macro_stage::validate_facts(transaction, &intent, &recovery)?;
        }
        if layout_version >= 13 {
            models::validate_facts(transaction, &intent, &recovery)?;
        }
    }
    Ok(())
}

pub(super) fn validate_migration_audit_facts(
    transaction: &Transaction<'_>,
) -> Result<(), ChainPostCloseError> {
    verify_br159_acquisition_catalog(transaction)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    validate_acquisition_chain_in_transaction(transaction)
        .map_err(|_| ChainPostCloseError::SchemaRejected)
}

struct LoadedBegin {
    code: String,
    owner: String,
    generation: u64,
    run_version: u64,
    begun_at: i64,
}

fn load_begin(
    connection: &Connection,
    intent_id: &IntentId,
    ordinal: u64,
) -> Result<Option<LoadedBegin>, ChainPostCloseError> {
    let row = connection
        .query_row(
            "SELECT effect_key,request_codec_version,request_bytes,request_length, \
             request_sha256,lease_owner,lease_generation,run_version,begun_at \
             FROM chain_post_close_stage_begins \
             WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=?2",
            params![intent_id.as_str(), ordinal],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("begin read"))?;
    row.map(
        |(code, codec, bytes, length, digest, owner, generation, run_version, begun_at)| {
            if codec != 1
                || bytes != code.as_bytes()
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || generation < 1
                || run_version < 1
                || begun_at < 0
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(LoadedBegin {
                code,
                owner,
                generation: u64::try_from(generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(run_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                begun_at,
            })
        },
    )
    .transpose()
}

fn load_result(
    connection: &Connection,
    intent_id: &IntentId,
    ordinal: u64,
) -> Result<Option<StoredConceptProviderResult>, ChainPostCloseError> {
    let row = connection
        .query_row(
            "SELECT begun.effect_key,result.outcome,result.result_codec_version, \
                    result.result_bytes,result.result_length,result.result_sha256, \
                    result.run_version \
             FROM chain_post_close_stage_results AS result \
             JOIN chain_post_close_stage_begins AS begun \
               ON begun.intent_id=result.intent_id \
              AND begun.effect_kind=result.effect_kind \
              AND begun.effect_ordinal=result.effect_ordinal \
             WHERE result.intent_id=?1 AND result.effect_kind='ConceptProvider' \
               AND result.effect_ordinal=?2",
            params![intent_id.as_str(), ordinal],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("result read"))?;
    row.map(
        |(code, outcome, codec, bytes, length, digest, run_version)| {
            if !matches!(outcome.as_str(), "Returned" | "BusinessError")
                || codec != 1
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || run_version < 1
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(StoredConceptProviderResult {
                ordinal,
                code,
                outcome,
                bytes,
                run_version: u64::try_from(run_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
            })
        },
    )
    .transpose()
}

fn read_begins(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Vec<StageBeginFact>, ChainPostCloseError> {
    let mut statement = connection
        .prepare(
            "SELECT effect_ordinal FROM chain_post_close_stage_begins \
             WHERE intent_id=?1 ORDER BY effect_ordinal",
        )
        .map_err(|_| storage("begins"))?;
    let ordinals = statement
        .query_map([intent_id.as_str()], |row| row.get::<_, i64>(0))
        .map_err(|_| storage("begins"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("begins"))?;
    ordinals
        .into_iter()
        .map(|ordinal| {
            let ordinal =
                u64::try_from(ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
            let begin = load_begin(connection, intent_id, ordinal)?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            Ok(StageBeginFact {
                ordinal,
                code: begin.code,
                owner: begin.owner,
                generation: begin.generation,
                run_version: begin.run_version,
                begun_at: begin.begun_at,
            })
        })
        .collect()
}

fn read_results(
    connection: &Connection,
    intent_id: &IntentId,
    begins: &[StageBeginFact],
) -> Result<Vec<StageResultFact>, ChainPostCloseError> {
    let mut statement = connection
        .prepare(
            "SELECT effect_ordinal,lease_owner,lease_generation,run_version, \
                    returned_at,committed_at \
             FROM chain_post_close_stage_results \
             WHERE intent_id=?1 ORDER BY effect_ordinal",
        )
        .map_err(|_| storage("results"))?;
    let rows = statement
        .query_map([intent_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|_| storage("results"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("results"))?;
    rows.into_iter()
        .map(
            |(ordinal, owner, generation, run_version, returned_at, committed_at)| {
                let ordinal =
                    u64::try_from(ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let run_version =
                    u64::try_from(run_version).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                let begin = begins
                    .iter()
                    .find(|begin| begin.ordinal == ordinal)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                if run_version <= begin.run_version
                    || owner != begin.owner
                    || u64::try_from(generation).ok() != Some(begin.generation)
                    || returned_at < begin.begun_at
                    || committed_at < returned_at
                {
                    return Err(ChainPostCloseError::SchemaRejected);
                }
                let result = load_result(connection, intent_id, ordinal)?
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                Ok(StageResultFact {
                    ordinal,
                    outcome: result.outcome,
                    bytes: result.bytes,
                    owner,
                    generation: begin.generation,
                    run_version: result.run_version,
                    committed_at,
                })
            },
        )
        .collect()
}

fn load_cache_write(
    connection: &Connection,
    intent_id: &IntentId,
    ordinal: u64,
) -> Result<Option<StoredConceptCacheWrite>, ChainPostCloseError> {
    let row = connection
        .query_row(
            "SELECT code,provider_result_run_version,concepts_codec_version,concepts_bytes, \
                    concepts_length,concepts_sha256,cache_updated_at,lease_owner, \
                    lease_generation,run_version,written_at \
             FROM chain_post_close_concept_cache_writes \
             WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=?2",
            params![intent_id.as_str(), ordinal],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("cache fact read"))?;
    row.map(
        |(
            code,
            provider_version,
            codec,
            bytes,
            length,
            digest,
            cache_updated_at,
            owner,
            generation,
            run_version,
            written_at,
        )| {
            let concepts = serde_json::from_slice::<Vec<String>>(&bytes)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            let timestamp =
                chrono::NaiveDateTime::parse_from_str(&cache_updated_at, "%Y-%m-%d %H:%M:%S")
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            if timestamp.format("%Y-%m-%d %H:%M:%S").to_string() != cache_updated_at
                || codec != 1
                || concepts.is_empty()
                || concepts
                    .iter()
                    .any(|concept| concept.trim().is_empty() || concept.contains('\0'))
                || serde_json::to_vec(&concepts).ok().as_deref() != Some(bytes.as_slice())
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || owner.trim().is_empty()
                || generation < 1
                || provider_version < 1
                || run_version <= provider_version
                || written_at < 0
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(StoredConceptCacheWrite {
                ordinal,
                code,
                provider_result_run_version: u64::try_from(provider_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                concepts,
                owner,
                generation: u64::try_from(generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(run_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                written_at,
            })
        },
    )
    .transpose()
}

fn inspect_concept_batch_on(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
) -> Result<ConceptBatchInspection, ChainPostCloseError> {
    let recovery = inspect_run_on(connection, intent_id)?;
    inspect_concept_batch_from_recovery_on(connection, intent_id, &recovery)
}

fn inspect_concept_batch_from_recovery_on(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &RunRecovery,
) -> Result<ConceptBatchInspection, ChainPostCloseError> {
    let mut concepts = HashMap::new();
    for row in recovery.input.cache_rows() {
        let cached = recovery
            .input
            .cached_concepts(row.code())?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        concepts.insert(row.code().to_owned(), cached);
    }
    let missing = recovery
        .input
        .stocks()
        .iter()
        .filter_map(|stock| match recovery.input.cached_concepts(&stock.code) {
            Ok(None) => Some(Ok(stock.code.clone())),
            Ok(Some(_)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    for begin in &recovery.begins {
        let ordinal =
            usize::try_from(begin.ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if missing.get(ordinal) != Some(&begin.code) {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }
    for result in &recovery.results {
        let ordinal =
            usize::try_from(result.ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if missing.get(ordinal).is_none() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }

    let mut effects = Vec::with_capacity(missing.len());
    let mut provider_complete = true;
    for (ordinal, code) in missing.iter().enumerate() {
        let ordinal = u64::try_from(ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let begin = recovery.begins.iter().find(|fact| fact.ordinal == ordinal);
        let result = recovery.results.iter().find(|fact| fact.ordinal == ordinal);
        let state = match (begin, result) {
            (None, None) => {
                provider_complete = false;
                ConceptEffectRecoveryState::NeverStarted
            }
            (Some(_), None) => {
                provider_complete = false;
                ConceptEffectRecoveryState::BegunUnconfirmed
            }
            (Some(_), Some(result)) => {
                if result.outcome == "Returned" {
                    let raw = std::str::from_utf8(&result.bytes)
                        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                    if let Ok(value) = parse_concept_provider_raw(raw, code) {
                        concepts.insert(code.clone(), value);
                    } else {
                        provider_complete = false;
                    }
                } else {
                    provider_complete = false;
                }
                ConceptEffectRecoveryState::Confirmed
            }
            (None, Some(_)) => return Err(ChainPostCloseError::SchemaRejected),
        };
        effects.push(ConceptEffectRecovery {
            ordinal,
            code: code.clone(),
            state,
        });
    }

    let mut statement = connection
        .prepare(
            "SELECT effect_ordinal FROM chain_post_close_concept_cache_writes \
             WHERE intent_id=?1 ORDER BY run_version",
        )
        .map_err(|_| storage("cache facts"))?;
    let ordinals = statement
        .query_map([intent_id.as_str()], |row| row.get::<_, i64>(0))
        .map_err(|_| storage("cache facts"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("cache facts"))?;
    let mut applied_codes = Vec::with_capacity(ordinals.len());
    let mut fact_versions = recovery
        .begins
        .iter()
        .map(|fact| fact.run_version)
        .chain(recovery.results.iter().map(|fact| fact.run_version))
        .collect::<HashSet<_>>();
    for ordinal in ordinals {
        let ordinal = u64::try_from(ordinal).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let expected_code = usize::try_from(ordinal)
            .ok()
            .and_then(|ordinal| missing.get(ordinal))
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let applied = load_cache_write(connection, intent_id, ordinal)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let result = recovery
            .results
            .iter()
            .find(|result| result.ordinal == ordinal)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let raw =
            std::str::from_utf8(&result.bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let result_concepts = parse_concept_provider_raw(raw, expected_code)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if result.outcome != "Returned"
            || &applied.code != expected_code
            || result.run_version != applied.provider_result_run_version
            || applied.concepts != result_concepts
            || applied.generation < result.generation
            || applied.generation > recovery.generation
            || (applied.generation == result.generation && applied.owner != result.owner)
            || (applied.generation == recovery.generation && applied.owner != recovery.owner)
            || applied.written_at < result.committed_at
            || applied.written_at > recovery.updated_at
            || applied.run_version > recovery.head
            || !fact_versions.insert(applied.run_version)
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        applied_codes.push(applied.code);
    }
    let all_successes_applied = recovery
        .results
        .iter()
        .filter(|result| result.outcome == "Returned")
        .all(|result| {
            usize::try_from(result.ordinal)
                .ok()
                .and_then(|ordinal| missing.get(ordinal))
                .is_some_and(|code| applied_codes.iter().any(|applied| applied == code))
        });
    Ok(ConceptBatchInspection {
        concepts,
        applied_codes,
        effects,
        complete: provider_complete && all_successes_applied,
    })
}

fn check_lease(
    connection: &Connection,
    lease: &RunLease,
    now: UtcMicros,
) -> Result<(), ChainPostCloseError> {
    let (run_id, owner, generation, head, until): (String, String, i64, i64, i64) = connection
        .query_row(
            "SELECT run_id,lease_owner,lease_generation,head_version,lease_until \
             FROM chain_post_close_runs WHERE intent_id=?1",
            [lease.intent_id.as_str()],
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
        .optional()
        .map_err(|_| storage("lease check"))?
        .ok_or(ChainPostCloseError::RunMissing)?;
    if run_id != lease.run_id.as_str()
        || owner != lease.owner.as_str()
        || u64::try_from(generation).ok() != Some(lease.generation)
        || u64::try_from(head).ok() != Some(lease.head)
    {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    if now.get() >= until || now >= lease.until {
        return Err(ChainPostCloseError::LeaseExpired {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    Ok(())
}

fn validate_missing_request(
    input: &FixedChainPreparationInput,
    request: &ConceptProviderRequest,
) -> Result<(), ChainPostCloseError> {
    let missing = input
        .stocks()
        .iter()
        .filter_map(|stock| match input.cached_concepts(&stock.code) {
            Ok(None) => Some(Ok(stock.code.as_str())),
            Ok(Some(_)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ordinal =
        usize::try_from(request.ordinal).map_err(|_| ChainPostCloseError::InvalidInput {
            check: "missing concept request",
        })?;
    if missing.get(ordinal).copied() != Some(request.code.as_str()) {
        return Err(ChainPostCloseError::InvalidInput {
            check: "missing concept request",
        });
    }
    Ok(())
}

fn validate_first_missing_request(
    input: &FixedChainPreparationInput,
    request: &ConceptProviderRequest,
) -> Result<(), ChainPostCloseError> {
    if request.ordinal != 0 {
        return Err(ChainPostCloseError::InvalidInput {
            check: "first missing concept request",
        });
    }
    validate_missing_request(input, request).map_err(|_| ChainPostCloseError::InvalidInput {
        check: "first missing concept request",
    })
}

struct EffectGuard {
    cancelled: Rc<Cell<bool>>,
    armed: bool,
}

impl EffectGuard {
    fn new(cancelled: Rc<Cell<bool>>) -> Self {
        Self {
            cancelled,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for EffectGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.set(true);
        }
    }
}

pub(crate) struct LocalConceptPreparationIo<'local, 'provider, 'store, P, C> {
    local: &'local mut LocalChainPostClose<'store>,
    lease: Option<RunLease>,
    provider: &'provider mut P,
    clock: &'provider C,
    cancelled: Rc<Cell<bool>>,
    intent_id: String,
}

#[async_trait::async_trait(?Send)]
impl<P, C> ChainPreparationIo for LocalConceptPreparationIo<'_, '_, '_, P, C>
where
    P: ConceptProviderRawIo,
    C: ConceptEffectClock,
{
    fn validate_fixed_input(
        &mut self,
        business_date: NaiveDate,
        limit_ups: &[TopStock],
        macro_news: &Option<String>,
    ) -> AnyResult<()> {
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let lease = self.lease.as_ref().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        let stocks_match =
            serde_json::to_vec(limit_ups).ok() == serde_json::to_vec(lease.input.stocks()).ok();
        if business_date.format("%Y-%m-%d").to_string() != lease.input.business_date()
            || !stocks_match
            || macro_news != lease.input.macro_news()
        {
            anyhow::bail!("public preparation input does not match the fixed run");
        }
        Ok(())
    }

    async fn concepts(&mut self, codes: &[String]) -> AnyResult<HashMap<String, Vec<String>>> {
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: lease.intent_id.as_str().to_owned(),
            }
            .into());
        }
        let requested_codes = lease
            .input
            .stocks()
            .iter()
            .map(|stock| stock.code.as_str())
            .collect::<Vec<_>>();
        if requested_codes != codes.iter().map(String::as_str).collect::<Vec<_>>() {
            anyhow::bail!("concept codes do not match the fixed run");
        }
        let mut cached = HashMap::new();
        let mut missing = None;
        for code in codes {
            match lease
                .input
                .cached_concepts(code)
                .map_err(anyhow::Error::new)?
            {
                Some(concepts) => {
                    cached.insert(code.clone(), concepts);
                }
                None if missing.is_none() => missing = Some(code.clone()),
                None => {}
            }
        }
        let Some(code) = missing else {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::ConceptCacheConsumption,
            }
            .into());
        };
        let request = ConceptProviderRequest::try_new(0, code.clone())
            .map_err(|_| anyhow::anyhow!("fixed concept request is invalid"))?;
        let (next_lease, admission) = self
            .local
            .begin_concept_provider(lease, request, self.clock.now())
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        lease = next_lease;
        let stored = match admission {
            ConceptProviderAdmission::Replay(stored) => stored,
            ConceptProviderAdmission::Call(call) => {
                let mut guard = EffectGuard::new(Rc::clone(&self.cancelled));
                let intent_id = lease.intent_id.as_str().to_owned();
                let raw = self.provider.call_raw(&code).await;
                let outcome = match raw {
                    Ok(value) => ConceptProviderRawResult::Returned(value),
                    Err(error) => ConceptProviderRawResult::BusinessError(error),
                };
                let (next_lease, stored) = self
                    .local
                    .record_concept_provider_result(lease, call, outcome, self.clock.now())
                    .map_err(|error| match error {
                        ChainPostCloseError::LeaseExpired { .. }
                        | ChainPostCloseError::StaleLease { .. }
                        | ChainPostCloseError::LeaseHeld { .. } => {
                            preparation_stop(error, &self.intent_id)
                        }
                        _ => result_unconfirmed(error, intent_id.clone()),
                    })?;
                lease = next_lease;
                guard.disarm();
                stored
            }
        };
        self.lease = Some(lease);
        if stored.outcome == "BusinessError" {
            return Err(anyhow::anyhow!(
                "{}",
                String::from_utf8(stored.bytes)
                    .map_err(|_| anyhow::anyhow!("concept provider error bytes are invalid"))?
            ));
        }
        let raw = String::from_utf8(stored.bytes)
            .map_err(|_| anyhow::anyhow!("concept provider result bytes are invalid"))?;
        cached.insert(
            code.clone(),
            parse_concept_provider_raw(&raw, &code).map_err(anyhow::Error::msg)?,
        );
        Err(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ConceptCacheWrite,
        }
        .into())
    }
}

type ConceptCallFuture<'provider> = Pin<
    Box<
        dyn Future<
                Output = (
                    ConceptProviderCall,
                    String,
                    std::result::Result<String, String>,
                    EffectGuard,
                ),
            > + 'provider,
    >,
>;

fn concept_call_future<'provider>(
    provider: &'provider dyn ConceptProviderRawIo,
    call: ConceptProviderCall,
    code: String,
    cancelled: Rc<Cell<bool>>,
) -> ConceptCallFuture<'provider> {
    let guard = EffectGuard::new(cancelled);
    Box::pin(async move {
        let raw = provider.call_raw(&code).await;
        (call, code, raw, guard)
    })
}

#[derive(Clone, Copy)]
enum ConceptSource<'provider> {
    Raw(&'provider dyn ConceptProviderRawIo),
    Rpc(&'provider ConnectedBoardQueries),
}

#[derive(Clone, Copy)]
enum BoardSource<'provider> {
    Legacy(&'provider GrpcSource),
    Connected(&'provider ConnectedBoardQueries),
}

#[derive(Clone, Copy)]
enum MacroLayout {
    V11,
    V12,
}

struct MacroInput<'provider> {
    layout: MacroLayout,
    source: &'provider GrpcSource,
    clock: &'provider dyn MacroObservationClock,
    search_service: &'provider SearchService,
}

/// Live adapters for the ModelsSearchAndReport stage. Present only on v13.
struct ModelsInput<'provider> {
    analyzer: &'provider crate::analyzer::GeminiAnalyzer,
    search_service: &'provider SearchService,
    clock: &'provider dyn ModelsObservationClock,
    session: Option<ModelsSession>,
}

/// Per-run replay cursor over the journaled effects, opened by
/// `before_models_search_and_report`.
struct ModelsSession {
    replay: Vec<(models::Request, Option<models::Outcome>)>,
    next_ordinal: u64,
    /// A store failure inside an infallible hook (`model_available`,
    /// `search_available`, `local_now`) is parked here and surfaced by the next
    /// fallible hook, so the stage still fails closed.
    pending_stop: Option<anyhow::Error>,
}

enum ModelsStep {
    Replayed(models::Outcome),
    Begun(models::Begun),
}

pub(crate) struct LocalConceptBatchPreparationIo<'local, 'provider, 'store, C> {
    local: &'local mut LocalChainPostClose<'store>,
    lease: Option<RunLease>,
    concept_source: ConceptSource<'provider>,
    clock: &'provider C,
    cancelled: Rc<Cell<bool>>,
    intent_id: String,
    configuration: Option<FixedClusterConfiguration>,
    board_source: Option<BoardSource<'provider>>,
    position_observation_clock: Option<&'provider dyn PositionObservationClock>,
    position_rpc: bool,
    dragon_tiger_source: Option<&'provider GrpcSource>,
    dragon_tiger_clock: Option<&'provider dyn DragonTigerObservationClock>,
    macro_input: Option<MacroInput<'provider>>,
    models_input: Option<ModelsInput<'provider>>,
}

#[async_trait::async_trait(?Send)]
impl<C> ChainPreparationIo for LocalConceptBatchPreparationIo<'_, '_, '_, C>
where
    C: ConceptEffectClock,
{
    fn validate_fixed_input(
        &mut self,
        business_date: NaiveDate,
        limit_ups: &[TopStock],
        macro_news: &Option<String>,
    ) -> AnyResult<()> {
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let lease = self.lease.as_ref().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        let stocks_match =
            serde_json::to_vec(limit_ups).ok() == serde_json::to_vec(lease.input.stocks()).ok();
        if business_date.format("%Y-%m-%d").to_string() != lease.input.business_date()
            || !stocks_match
            || macro_news != lease.input.macro_news()
        {
            anyhow::bail!("public preparation input does not match the fixed run");
        }
        Ok(())
    }

    async fn concepts(&mut self, codes: &[String]) -> AnyResult<HashMap<String, Vec<String>>> {
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        let requested_codes = lease
            .input
            .stocks()
            .iter()
            .map(|stock| stock.code.as_str())
            .collect::<Vec<_>>();
        if requested_codes != codes.iter().map(String::as_str).collect::<Vec<_>>() {
            anyhow::bail!("concept codes do not match the fixed run");
        }

        let initial = self
            .local
            .inspect_concept_batch(&lease.intent_id)
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        if initial
            .effects()
            .iter()
            .any(|effect| effect.state() == ConceptEffectRecoveryState::BegunUnconfirmed)
        {
            // Same two-layer shape as the macro drivers for an unconfirmed effect on
            // reopen: the domain error is the root cause, the stop is its context.
            return Err(preparation_stop(
                ChainPostCloseError::IncompleteEffect {
                    intent_id: self.intent_id.clone(),
                },
                &self.intent_id,
            ));
        }
        let missing = lease
            .input
            .stocks()
            .iter()
            .filter_map(|stock| match lease.input.cached_concepts(&stock.code) {
                Ok(None) => Some(Ok(stock.code.clone())),
                Ok(Some(_)) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        if let ConceptSource::Rpc(queries) = self.concept_source {
            let (next_lease, stored) = concept_rpc_driver::drive(
                self.local,
                lease,
                queries,
                self.clock,
                concept_rpc_driver::Journal::Initial,
                &missing
                    .iter()
                    .enumerate()
                    .map(|(ordinal, code)| (ordinal as u64, code.clone()))
                    .collect::<Vec<_>>(),
                Rc::clone(&self.cancelled),
            )
            .await?;
            lease = next_lease;
            let mut stored = stored
                .into_iter()
                .map(concept_rpc_driver::Projection::into_initial)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| preparation_stop(error, &self.intent_id))?;
            stored.sort_by_key(|result| result.run_version);
            let mut concepts = initial.concepts().clone();
            for result in &stored {
                if result.outcome == "BusinessError" {
                    self.lease = Some(lease);
                    return Err(anyhow::anyhow!(
                        "{}",
                        String::from_utf8(result.bytes.clone()).map_err(|_| {
                            anyhow::anyhow!("concept provider error bytes are invalid")
                        })?
                    ));
                }
                let raw = std::str::from_utf8(&result.bytes)
                    .map_err(|_| anyhow::anyhow!("concept provider result bytes are invalid"))?;
                parse_concept_provider_raw(raw, &result.code).map_err(anyhow::Error::msg)?;
                let (next_lease, parsed) = self
                    .local
                    .commit_concept_cache_write(lease, result, self.clock.now())
                    .map_err(|error| preparation_stop(error, &self.intent_id))?;
                lease = next_lease;
                concepts.insert(result.code.clone(), parsed);
            }
            self.lease = Some(lease);
            return Ok(concepts);
        }
        let mut stored = Vec::with_capacity(missing.len());
        let mut futures = FuturesUnordered::<ConceptCallFuture<'_>>::new();
        let mut next = 0usize;

        while next < missing.len() || !futures.is_empty() {
            while next < missing.len() && futures.len() < 6 {
                let ordinal =
                    u64::try_from(next).map_err(|_| anyhow::anyhow!("concept ordinal overflow"))?;
                let code = missing[next].clone();
                let request = ConceptProviderRequest::try_new(ordinal, code.clone())
                    .map_err(|_| anyhow::anyhow!("fixed concept request is invalid"))?;
                let (next_lease, admission) = self
                    .local
                    .begin_concept_provider(lease, request, self.clock.now())
                    .map_err(|error| preparation_stop(error, &self.intent_id))?;
                lease = next_lease;
                match admission {
                    ConceptProviderAdmission::Replay(result) => stored.push(result),
                    ConceptProviderAdmission::Call(call) => {
                        let ConceptSource::Raw(provider) = self.concept_source else {
                            unreachable!("RPC source returned before its dedicated path")
                        };
                        futures.push(concept_call_future(
                            provider,
                            call,
                            code,
                            Rc::clone(&self.cancelled),
                        ));
                    }
                }
                next += 1;
            }
            let Some((call, _code, raw, mut guard)) = futures.next().await else {
                continue;
            };
            let outcome = match raw {
                Ok(value) => ConceptProviderRawResult::Returned(value),
                Err(error) => ConceptProviderRawResult::BusinessError(error),
            };
            let intent_id = self.intent_id.clone();
            let (next_lease, result) = self
                .local
                .record_concept_provider_result(lease, call, outcome, self.clock.now())
                .map_err(|error| match error {
                    ChainPostCloseError::LeaseExpired { .. }
                    | ChainPostCloseError::StaleLease { .. }
                    | ChainPostCloseError::LeaseHeld { .. } => {
                        preparation_stop(error, &self.intent_id)
                    }
                    _ => result_unconfirmed(error, intent_id.clone()),
                })?;
            lease = next_lease;
            guard.disarm();
            stored.push(result);
        }

        stored.sort_by_key(|result| result.run_version);
        let mut concepts = initial.concepts().clone();
        for result in &stored {
            if result.outcome == "BusinessError" {
                self.lease = Some(lease);
                return Err(anyhow::anyhow!(
                    "{}",
                    String::from_utf8(result.bytes.clone())
                        .map_err(|_| anyhow::anyhow!("concept provider error bytes are invalid"))?
                ));
            }
            let raw = std::str::from_utf8(&result.bytes)
                .map_err(|_| anyhow::anyhow!("concept provider result bytes are invalid"))?;
            parse_concept_provider_raw(raw, &result.code).map_err(anyhow::Error::msg)?;
            let (next_lease, parsed) = self
                .local
                .commit_concept_cache_write(lease, result, self.clock.now())
                .map_err(|error| preparation_stop(error, &self.intent_id))?;
            lease = next_lease;
            concepts.insert(result.code.clone(), parsed);
        }
        self.lease = Some(lease);
        Ok(concepts)
    }

    fn before_cluster_configuration(
        &mut self,
        concepts: &HashMap<String, Vec<String>>,
    ) -> AnyResult<()> {
        let intent_id = self
            .lease
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(PreparationStop::AuthorityRejected {
                    intent_id: self.intent_id.clone(),
                })
            })?
            .intent_id
            .clone();
        let inspection = self
            .local
            .inspect_concept_batch(&intent_id)
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        if !inspection.is_complete() || inspection.concepts() != concepts {
            return Err(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        Err(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        }
        .into())
    }

    fn cluster_material(
        &mut self,
        stocks: &[TopStock],
        concepts: &HashMap<String, Vec<String>>,
    ) -> AnyResult<(
        usize,
        Vec<crate::pipeline::chain_analysis::ChainCluster>,
        Vec<TopStock>,
    )> {
        let Some(configuration) = self.configuration else {
            self.before_cluster_configuration(concepts)?;
            unreachable!("v3 adapter always stops before cluster configuration")
        };
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        let existing = self
            .local
            .load_cluster_material(&lease, concepts)
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        let (clusters, isolated) = if let Some(material) = existing {
            material
        } else {
            let material =
                build_cluster_material(stocks, concepts, configuration.min_cluster_size());
            lease = self
                .local
                .record_cluster_material(
                    lease,
                    concepts,
                    &material.0,
                    &material.1,
                    self.clock.now(),
                )
                .map_err(|error| preparation_stop(error, &self.intent_id))?;
            material
        };
        self.lease = Some(lease);
        Ok((configuration.min_cluster_size(), clusters, isolated))
    }

    async fn persist_clusters(
        &mut self,
        date: NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> AnyResult<HashMap<String, i64>> {
        if self.configuration.is_none() {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::ClusterConfiguration,
            }
            .into());
        }
        let lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        let (lease, lifecycle) = self
            .local
            .apply_chain_daily(lease, date, rows, self.clock.now())
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        self.lease = Some(lease);
        Ok(lifecycle)
    }

    async fn board_codes(
        &mut self,
    ) -> AnyResult<(
        HashMap<String, String>,
        Vec<crate::data_gateway::BatchEvidence>,
    )> {
        let Some(source) = self.board_source else {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::BoardDirectory,
            }
            .into());
        };
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        if let Some((codes, evidence)) = self
            .local
            .load_board_directory(&lease)
            .map_err(|error| preparation_stop(error, &self.intent_id))?
        {
            self.lease = Some(lease);
            return Ok((codes.into_iter().collect(), evidence));
        }
        let mut codes = HashMap::new();
        let mut evidence = Vec::new();
        for kind in [
            crate::data_gateway::BoardKind::Industry,
            crate::data_gateway::BoardKind::Concept,
        ] {
            let batch = if let Some(stored) = self
                .local
                .load_board_kind(&lease, kind)
                .map_err(|error| preparation_stop(error, &self.intent_id))?
            {
                stored
            } else {
                let pending = self
                    .local
                    .load_pending_board_attempt(&lease, kind, self.clock.now())
                    .map_err(|error| preparation_stop(error, &self.intent_id))?;
                match pending {
                    Some(board::PendingBoardAttempt::Response {
                        terminal,
                        profile,
                        acquisition_authority,
                        request_id,
                        response,
                    }) => {
                        let projected = GrpcSource::restore_board_directory_response(
                            profile,
                            acquisition_authority.as_deref(),
                            &request_id,
                            response,
                        )
                        .map_err(|_| {
                            preparation_stop(
                                ChainPostCloseError::IncompleteEffect {
                                    intent_id: self.intent_id.clone(),
                                },
                                &self.intent_id,
                            )
                        })?;
                        let projected = Ok(projected);
                        let (next_lease, stored) = self
                            .local
                            .finalize_board_kind(lease, &terminal, &projected, self.clock.now())
                            .map_err(|error| result_unconfirmed(error, self.intent_id.clone()))?;
                        lease = next_lease;
                        stored
                    }
                    Some(board::PendingBoardAttempt::Error { terminal, material }) => {
                        let projected = Err(material.into_gateway_error());
                        let (next_lease, stored) = self
                            .local
                            .finalize_board_kind(lease, &terminal, &projected, self.clock.now())
                            .map_err(|error| result_unconfirmed(error, self.intent_id.clone()))?;
                        lease = next_lease;
                        stored
                    }
                    pending => {
                        let (mut session, resumed) = match pending {
                            Some(board::PendingBoardAttempt::Retry {
                                profile,
                                acquisition_authority,
                                request,
                                retry_policy,
                                next_attempt,
                                backoff_ms,
                            }) => {
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                let session = match source {
                                    BoardSource::Legacy(source) => {
                                        source
                                            .resume_board_directory_query_session(
                                                request,
                                                profile,
                                                acquisition_authority.as_deref(),
                                                retry_policy,
                                                next_attempt,
                                            )
                                            .await
                                    }
                                    BoardSource::Connected(queries) => queries
                                        .resume_directory_session(
                                            request,
                                            profile,
                                            acquisition_authority.as_deref(),
                                            retry_policy,
                                            next_attempt,
                                        ),
                                }
                                .map_err(|error| {
                                    anyhow::Error::new(error).context(
                                        PreparationStop::AuthorityRejected {
                                            intent_id: self.intent_id.clone(),
                                        },
                                    )
                                })?;
                                (session, true)
                            }
                            None => {
                                let session = match source {
                                    BoardSource::Legacy(source) => {
                                        source
                                            .board_directory_query_session(kind, board::BOARD_LIMIT)
                                            .await
                                    }
                                    BoardSource::Connected(queries) => {
                                        queries.directory_session(kind, board::BOARD_LIMIT)
                                    }
                                }
                                .map_err(anyhow::Error::new)?;
                                (session, false)
                            }
                            Some(
                                board::PendingBoardAttempt::Response { .. }
                                | board::PendingBoardAttempt::Error { .. },
                            ) => unreachable!(),
                        };
                        loop {
                            let authorized = session.authorize_next().map_err(|error| {
                                let error = anyhow::Error::new(error);
                                if resumed {
                                    error.context(PreparationStop::AuthorityRejected {
                                        intent_id: self.intent_id.clone(),
                                    })
                                } else {
                                    error
                                }
                            })?;
                            let (next_lease, call) = self
                                .local
                                .begin_board_attempt(lease, kind, &authorized, self.clock.now())
                                .map_err(|error| preparation_stop(error, &self.intent_id))?;
                            lease = next_lease;
                            let completion = authorized.execute().await;
                            let continuation = completion.continuation;
                            let intent_id = self.intent_id.clone();
                            let (next_lease, stored, error_capability) = self
                                .local
                                .record_board_attempt_result(
                                    lease,
                                    call,
                                    &completion,
                                    self.clock.now(),
                                )
                                .map_err(|error| result_unconfirmed(error, intent_id))?;
                            lease = next_lease;
                            match continuation {
                                crate::data_gateway::grpc_source::BoardContinuation::Retry {
                                    backoff_ms,
                                } => {
                                    tokio::time::sleep(std::time::Duration::from_millis(
                                        backoff_ms,
                                    ))
                                    .await;
                                }
                                crate::data_gateway::grpc_source::BoardContinuation::Terminal => {
                                    let projected =
                                        GrpcSource::board_directory_completion(completion);
                                    if projected.is_err() {
                                        let Some(capability) = error_capability else {
                                            // v5 intentionally retains the legacy finalization path.
                                            let (next_lease, stored) = self
                                                .local
                                                .finalize_board_kind(
                                                    lease,
                                                    &stored,
                                                    &projected,
                                                    self.clock.now(),
                                                )
                                                .map_err(|error| {
                                                    result_unconfirmed(
                                                        error,
                                                        self.intent_id.clone(),
                                                    )
                                                })?;
                                            lease = next_lease;
                                            break stored;
                                        };
                                        let (next_lease, _) = self
                                            .local
                                            .confirm_board_error_material(
                                                lease,
                                                capability,
                                                &projected,
                                                self.clock.now(),
                                            )
                                            .map_err(|error| {
                                                result_unconfirmed(error, self.intent_id.clone())
                                            })?;
                                        lease = next_lease;
                                    }
                                    let (next_lease, stored) = self
                                        .local
                                        .finalize_board_kind(
                                            lease,
                                            &stored,
                                            &projected,
                                            self.clock.now(),
                                        )
                                        .map_err(|error| {
                                            result_unconfirmed(error, self.intent_id.clone())
                                        })?;
                                    lease = next_lease;
                                    break stored;
                                }
                            }
                        }
                    }
                }
            };
            let batch = match batch {
                Ok(batch) => batch,
                Err(error) => {
                    self.lease = Some(lease);
                    return Err(anyhow::anyhow!("产业链板块目录不可用 ({kind:?}): {error}"));
                }
            };
            if let Err(error) = crate::pipeline::chain_analysis::fold_board_directory_kind(
                &mut codes,
                &mut evidence,
                kind,
                batch,
            ) {
                self.lease = Some(lease);
                return Err(anyhow::Error::msg(error));
            }
        }
        if codes.is_empty() {
            self.lease = Some(lease);
            anyhow::bail!("Magic TDX 板块目录没有可用记录");
        }
        let canonical = codes
            .iter()
            .map(|(name, code)| (name.clone(), code.clone()))
            .collect();
        lease = self
            .local
            .record_board_directory(lease, &canonical, &evidence, self.clock.now())
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        self.lease = Some(lease);
        Ok((codes, evidence))
    }

    async fn candidates(
        &mut self,
        board: &str,
        _excluded: &HashSet<String>,
    ) -> AnyResult<crate::data_gateway::GatewayBatch<TopStock>> {
        anyhow::bail!("产业链补涨候选 unsupported: board={board}; Magic TDX 当前只提供成分身份")
    }

    fn resolve_board_code(
        &mut self,
        cluster_ordinal: usize,
        cluster: &crate::pipeline::chain_analysis::ChainCluster,
        board_map: &HashMap<String, String>,
    ) -> AnyResult<String> {
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        if let Some(stored) = self
            .local
            .load_board_selection(&lease, cluster_ordinal, &cluster.concept)
            .map_err(|error| preparation_stop(error, &self.intent_id))?
        {
            self.lease = Some(lease);
            return stored.ok_or_else(|| {
                anyhow::anyhow!("产业链主线「{}」未匹配到概念板块代码", cluster.concept)
            });
        }
        let selected =
            crate::pipeline::chain_analysis::resolve_cluster_board_code_owned(cluster, board_map);
        let (next_lease, stored) = self
            .local
            .record_board_selection(
                lease,
                cluster_ordinal,
                &cluster.concept,
                selected.as_deref().ok(),
                self.clock.now(),
            )
            .map_err(|error| preparation_stop(error, &self.intent_id))?;
        lease = next_lease;
        self.lease = Some(lease);
        match (selected, stored) {
            (Ok(expected), Some(actual)) if expected == actual => Ok(actual),
            (Err(error), None) => Err(error),
            _ => Err(preparation_stop(
                ChainPostCloseError::SchemaRejected,
                &self.intent_id,
            )),
        }
    }

    async fn positions(
        &mut self,
    ) -> AnyResult<Vec<crate::pipeline::chain_analysis::preparation::PositionInput>> {
        let Some(clock) = self.position_observation_clock else {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Positions,
            }
            .into());
        };
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        match self.local.capture_or_load_positions(&lease, clock) {
            Ok((head, result)) => {
                lease.head = head;
                self.lease = Some(lease);
                Ok(result)
            }
            Err(
                error @ ChainPostCloseError::StorageFailed {
                    operation: "commit",
                },
            ) => {
                self.cancelled.set(true);
                Err(result_unconfirmed(error, self.intent_id.clone()))
            }
            Err(error) => {
                self.lease = Some(lease);
                Err(preparation_stop(error, &self.intent_id))
            }
        }
    }

    async fn position_concepts(
        &mut self,
        codes: &[String],
    ) -> AnyResult<HashMap<String, Vec<String>>> {
        let Some(clock) = self.position_observation_clock else {
            return self.concepts(codes).await;
        };
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let mut lease = self.lease.take().ok_or_else(|| {
            anyhow::anyhow!(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        let (head, material) = match self
            .local
            .capture_or_load_position_concepts(&lease, codes, clock)
        {
            Ok(value) => value,
            Err(
                error @ ChainPostCloseError::StorageFailed {
                    operation: "commit",
                },
            ) => {
                self.cancelled.set(true);
                return Err(result_unconfirmed(error, self.intent_id.clone()));
            }
            Err(error) => {
                self.lease = Some(lease);
                return Err(preparation_stop(error, &self.intent_id));
            }
        };
        lease.head = head;
        if self.position_rpc {
            let batch = match material.into_batch() {
                Ok(batch) => batch,
                Err(error) => {
                    self.lease = Some(lease);
                    return Err(anyhow::Error::msg(error));
                }
            };
            let ConceptSource::Rpc(queries) = self.concept_source else {
                return Err(preparation_stop(
                    ChainPostCloseError::SchemaRejected,
                    &self.intent_id,
                ));
            };
            let (next, projections) = concept_rpc_driver::drive(
                self.local,
                lease,
                queries,
                self.clock,
                concept_rpc_driver::Journal::Positions(&batch),
                &batch.work,
                Rc::clone(&self.cancelled),
            )
            .await?;
            lease = next;
            let mut projections = projections
                .into_iter()
                .map(concept_rpc_driver::Projection::into_positions)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| preparation_stop(error, &self.intent_id))?;
            projections.sort_by_key(position_concept_rpc::Projection::order);
            let mut concepts = batch.cached.clone();
            for projection in projections {
                if let Some(error) = projection.business_error() {
                    self.lease = Some(lease);
                    return Err(anyhow::Error::msg(error.to_owned()));
                }
                let (next, parsed) = self
                    .local
                    .apply_position_rpc_cache(lease, &batch, &projection, self.clock.now())
                    .map_err(|error| {
                        self.cancelled.set(true);
                        result_unconfirmed(error, self.intent_id.clone())
                    })?;
                lease = next;
                concepts.insert(projection.code().to_owned(), parsed);
            }
            self.lease = Some(lease);
            return Ok(concepts);
        }
        self.lease = Some(lease);
        let concepts = material.parse().map_err(anyhow::Error::msg)?;
        if codes.iter().any(|code| !concepts.contains_key(code)) {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::PositionConceptProvider,
            }
            .into());
        }
        Ok(concepts)
    }

    async fn lhb(
        &mut self,
    ) -> AnyResult<(
        HashMap<String, f64>,
        crate::pipeline::chain_analysis::preparation::SourceObservation,
    )> {
        if let Some(input) = &self.macro_input {
            let lease = self.lease.as_ref().ok_or_else(|| {
                anyhow::Error::new(PreparationStop::AuthorityRejected {
                    intent_id: self.intent_id.clone(),
                })
            })?;
            let projection = self
                .local
                .recover_macro_parent(lease, input.clock.now())
                .map_err(|error| preparation_stop(error, &self.intent_id))?;
            return match projection {
                dragon_tiger_codec::Projection::Available { lhb, source } => Ok((lhb, source)),
                dragon_tiger_codec::Projection::Failed(_) => {
                    Err(DragonTigerProjectionFailed.into())
                }
            };
        }
        if let (Some(source), Some(clock)) = (self.dragon_tiger_source, self.dragon_tiger_clock) {
            let lease = self.lease.take().ok_or_else(|| {
                anyhow::anyhow!(PreparationStop::AuthorityRejected {
                    intent_id: self.intent_id.clone()
                })
            })?;
            let (lease, projection) = dragon_tiger_driver::drive(
                self.local,
                lease,
                source,
                clock,
                Rc::clone(&self.cancelled),
            )
            .await?;
            self.lease = Some(lease);
            return match projection {
                dragon_tiger_codec::Projection::Available { lhb, source } => Ok((lhb, source)),
                dragon_tiger_codec::Projection::Failed(_) => {
                    Err(DragonTigerProjectionFailed.into())
                }
            };
        }
        if self.position_observation_clock.is_some() {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::DragonTiger,
            }
            .into());
        }
        Err(anyhow::anyhow!("dragon-tiger I/O not supplied"))
    }

    async fn macro_search(&mut self) -> AnyResult<String> {
        if let Some(input) = &self.macro_input {
            let lease = self.lease.take().ok_or_else(|| {
                anyhow::Error::new(PreparationStop::AuthorityRejected {
                    intent_id: self.intent_id.clone(),
                })
            })?;
            match input.layout {
                MacroLayout::V11 => {
                    let lease = macro_driver_v11::drive(
                        self.local,
                        lease,
                        input.source,
                        input.clock,
                        Rc::clone(&self.cancelled),
                        input.search_service,
                    )
                    .await?;
                    self.lease = Some(lease);
                    return Err(PreparationStop::StageNotMigrated {
                        next: UnmigratedStage::Macro,
                    }
                    .into());
                }
                MacroLayout::V12 => {
                    let (lease, output) = macro_driver::drive(
                        self.local,
                        lease,
                        input.source,
                        input.clock,
                        Rc::clone(&self.cancelled),
                        input.search_service,
                    )
                    .await?;
                    self.lease = Some(lease);
                    return Ok(output);
                }
            }
        }
        if self.dragon_tiger_source.is_some() {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Macro,
            }
            .into());
        }
        Err(anyhow::anyhow!("macro search I/O not supplied"))
    }

    async fn macro_search_with_budget(
        &mut self,
    ) -> std::result::Result<AnyResult<String>, tokio::time::error::Elapsed> {
        if self.macro_input.is_some() {
            Ok(self.macro_search().await)
        } else {
            tokio::time::timeout(std::time::Duration::from_secs(15), self.macro_search()).await
        }
    }

    fn before_models_search_and_report(&mut self) -> AnyResult<()> {
        if self.models_input.is_some() {
            let lease = self.lease.as_ref().ok_or_else(|| {
                anyhow::Error::new(PreparationStop::AuthorityRejected {
                    intent_id: self.intent_id.clone(),
                })
            })?;
            let recovery = self
                .local
                .load_models(lease, self.clock.now())
                .map_err(|error| preparation_stop(error, &self.intent_id))?;
            if recovery.begun_unconfirmed() {
                return Err(preparation_stop(
                    ChainPostCloseError::IncompleteEffect {
                        intent_id: self.intent_id.clone(),
                    },
                    &self.intent_id,
                ));
            }
            if let Some(input) = self.models_input.as_mut() {
                input.session = Some(ModelsSession {
                    replay: recovery.effects,
                    next_ordinal: 1,
                    pending_stop: None,
                });
            }
            return Ok(());
        }
        if self.dragon_tiger_source.is_some() {
            return Err(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::ModelsSearchAndReport,
            }
            .into());
        }
        Ok(())
    }

    fn model_available(&mut self) -> bool {
        let Some(analyzer) = self.models_input.as_ref().map(|input| input.analyzer) else {
            panic!("model configuration I/O not supplied")
        };
        match self.models_step(models::Request::ModelAvailable) {
            Ok(ModelsStep::Replayed(models::Outcome::Available(available))) => available,
            Ok(ModelsStep::Replayed(_)) => {
                let error = self.models_stop(ChainPostCloseError::SchemaRejected);
                self.models_park(error);
                false
            }
            Ok(ModelsStep::Begun(begun)) => {
                let available = analyzer.is_available();
                if let Err(error) = self.models_record(begun, &models::Outcome::Available(available))
                {
                    self.models_park(error);
                }
                available
            }
            Err(error) => {
                self.models_park(error);
                false
            }
        }
    }

    async fn model_effect(
        &mut self,
        effect: ModelEffect<'_>,
        prompt: &str,
        system: &str,
        mode: crate::analyzer::AgentMode,
    ) -> AnyResult<String> {
        let Some(analyzer) = self.models_input.as_ref().map(|input| input.analyzer) else {
            panic!("model I/O not supplied")
        };
        let request = models::Request::Model {
            stage: model_stage_name(effect.stage).to_owned(),
            concept: effect.concept.map(str::to_owned),
            prompt: prompt.to_owned(),
            system: system.to_owned(),
            mode: match mode {
                crate::analyzer::AgentMode::Quick => "Quick".to_owned(),
                crate::analyzer::AgentMode::Deep => "Deep".to_owned(),
            },
        };
        let begun = match self.models_step(request)? {
            ModelsStep::Replayed(models::Outcome::Returned(text)) => return Ok(text),
            ModelsStep::Replayed(models::Outcome::Failed(message)) => {
                return Err(anyhow::anyhow!(message));
            }
            ModelsStep::Replayed(_) => {
                return Err(self.models_stop(ChainPostCloseError::SchemaRejected));
            }
            ModelsStep::Begun(begun) => begun,
        };
        let mut guard = EffectGuard::new(Rc::clone(&self.cancelled));
        let result = analyzer.call_api_mode(prompt, system, mode).await;
        let outcome = match &result {
            Ok(text) => models::Outcome::Returned(text.clone()),
            Err(error) => models::Outcome::Failed(error.to_string()),
        };
        self.models_record(begun, &outcome)?;
        guard.disarm();
        result
    }

    fn search_available(&mut self) -> bool {
        let Some(search) = self
            .models_input
            .as_ref()
            .map(|input| input.search_service)
        else {
            panic!("search configuration I/O not supplied")
        };
        match self.models_step(models::Request::SearchAvailable) {
            Ok(ModelsStep::Replayed(models::Outcome::Available(available))) => available,
            Ok(ModelsStep::Replayed(_)) => {
                let error = self.models_stop(ChainPostCloseError::SchemaRejected);
                self.models_park(error);
                false
            }
            Ok(ModelsStep::Begun(begun)) => {
                let available = search.is_available();
                if let Err(error) = self.models_record(begun, &models::Outcome::Available(available))
                {
                    self.models_park(error);
                }
                available
            }
            Err(error) => {
                self.models_park(error);
                false
            }
        }
    }

    async fn search_effect(
        &mut self,
        stage: SearchStage,
        query: &str,
        limit: usize,
        budget: std::time::Duration,
    ) -> AnyResult<Vec<crate::search_service::SearchResult>> {
        let Some(search) = self
            .models_input
            .as_ref()
            .map(|input| input.search_service)
        else {
            panic!("topic search I/O not supplied")
        };
        let request = models::Request::Search {
            stage: match stage {
                SearchStage::Cluster => "Cluster".to_owned(),
                SearchStage::AfterMarket => "AfterMarket".to_owned(),
            },
            query: query.to_owned(),
            limit: limit as u64,
        };
        let begun = match self.models_step(request)? {
            ModelsStep::Replayed(models::Outcome::SearchReturned(results)) => return Ok(results),
            ModelsStep::Replayed(models::Outcome::Failed(message)) => {
                return Err(anyhow::anyhow!(message));
            }
            ModelsStep::Replayed(_) => {
                return Err(self.models_stop(ChainPostCloseError::SchemaRejected));
            }
            ModelsStep::Begun(begun) => begun,
        };
        // Finish strictly inside the renderer's outer budget so the result row
        // is always written; an outer timeout would drop this future mid-effect.
        let inner = budget
            .checked_sub(std::time::Duration::from_secs(1))
            .filter(|inner| !inner.is_zero())
            .unwrap_or(budget);
        let mut guard = EffectGuard::new(Rc::clone(&self.cancelled));
        let result = match tokio::time::timeout(inner, search.search_topic(query, limit)).await {
            Ok(results) => Ok(results),
            Err(_) => Err(anyhow::anyhow!(
                "新闻搜索超时（{}秒）",
                inner.as_secs()
            )),
        };
        let outcome = match &result {
            Ok(results) => models::Outcome::SearchReturned(results.clone()),
            Err(error) => models::Outcome::Failed(error.to_string()),
        };
        self.models_record(begun, &outcome)?;
        guard.disarm();
        result
    }

    fn local_now(&mut self) -> chrono::DateTime<chrono::FixedOffset> {
        let Some(clock) = self.models_input.as_ref().map(|input| input.clock) else {
            panic!("clock I/O not supplied")
        };
        let live = clock.models_local_observation();
        match self.models_step(models::Request::LocalNow) {
            Ok(ModelsStep::Replayed(models::Outcome::LocalNow(text))) => {
                match chrono::DateTime::parse_from_rfc3339(&text) {
                    Ok(stored) => stored,
                    Err(_) => {
                        let error = self.models_stop(ChainPostCloseError::SchemaRejected);
                        self.models_park(error);
                        live
                    }
                }
            }
            Ok(ModelsStep::Replayed(_)) => {
                let error = self.models_stop(ChainPostCloseError::SchemaRejected);
                self.models_park(error);
                live
            }
            Ok(ModelsStep::Begun(begun)) => {
                if let Err(error) =
                    self.models_record(begun, &models::Outcome::LocalNow(live.to_rfc3339()))
                {
                    self.models_park(error);
                }
                live
            }
            Err(error) => {
                self.models_park(error);
                live
            }
        }
    }

    fn commit_prepared(&mut self, prepared: &PreparedChainAnalysis) -> AnyResult<()> {
        if self.models_input.is_none() {
            return Ok(());
        }
        if let Some(error) = self
            .models_input
            .as_mut()
            .and_then(|input| input.session.as_mut())
            .and_then(|session| session.pending_stop.take())
        {
            return Err(error);
        }
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let artifact = prepared.to_artifact_bytes()?;
        let lease = self.lease.take().ok_or_else(|| {
            anyhow::Error::new(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        match self.local.finalize_models_stage(
            lease,
            &artifact,
            prepared.report().as_bytes(),
            self.clock.now(),
        ) {
            Ok(lease) => {
                self.lease = Some(lease);
                Ok(())
            }
            Err(error) => Err(result_unconfirmed(error, self.intent_id.clone())),
        }
    }
}

fn model_stage_name(stage: ModelStage) -> &'static str {
    match stage {
        ModelStage::SearchTerms => "SearchTerms",
        ModelStage::Deep => "Deep",
        ModelStage::Simple => "Simple",
        ModelStage::Overview => "Overview",
        ModelStage::UnselectedCluster => "UnselectedCluster",
    }
}

impl<C> LocalConceptBatchPreparationIo<'_, '_, '_, C>
where
    C: ConceptEffectClock,
{
    fn models_stop(&self, error: ChainPostCloseError) -> anyhow::Error {
        preparation_stop(error, &self.intent_id)
    }

    fn models_park(&mut self, error: anyhow::Error) {
        if let Some(session) = self
            .models_input
            .as_mut()
            .and_then(|input| input.session.as_mut())
        {
            if session.pending_stop.is_none() {
                session.pending_stop = Some(error);
            }
        }
    }

    /// Replays the journaled outcome for the next ordinal, or journals a new
    /// begin row for a live effect. Request drift against the journal is refused.
    fn models_step(&mut self, request: models::Request) -> AnyResult<ModelsStep> {
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.intent_id.clone(),
            }
            .into());
        }
        let (ordinal, replayed) = {
            let session = self
                .models_input
                .as_mut()
                .and_then(|input| input.session.as_mut())
                .ok_or_else(|| {
                    anyhow::Error::new(PreparationStop::AuthorityRejected {
                        intent_id: self.intent_id.clone(),
                    })
                })?;
            if let Some(error) = session.pending_stop.take() {
                return Err(error);
            }
            let ordinal = session.next_ordinal;
            session.next_ordinal += 1;
            let replayed = usize::try_from(ordinal - 1)
                .ok()
                .and_then(|index| session.replay.get(index))
                .map(|(stored, outcome)| (*stored == request, outcome.clone()));
            (ordinal, replayed)
        };
        if let Some((matches, outcome)) = replayed {
            if !matches {
                return Err(self.models_stop(ChainPostCloseError::SchemaRejected));
            }
            return match outcome {
                Some(outcome) => Ok(ModelsStep::Replayed(outcome)),
                None => Err(self.models_stop(ChainPostCloseError::IncompleteEffect {
                    intent_id: self.intent_id.clone(),
                })),
            };
        }
        let lease = self.lease.take().ok_or_else(|| {
            anyhow::Error::new(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        match self
            .local
            .begin_models_effect(lease, ordinal, &request, self.clock.now())
        {
            Ok((lease, begun)) => {
                self.lease = Some(lease);
                Ok(ModelsStep::Begun(begun))
            }
            Err(error) => Err(self.models_stop(error)),
        }
    }

    fn models_record(&mut self, begun: models::Begun, outcome: &models::Outcome) -> AnyResult<()> {
        let lease = self.lease.take().ok_or_else(|| {
            anyhow::Error::new(PreparationStop::AuthorityRejected {
                intent_id: self.intent_id.clone(),
            })
        })?;
        match self
            .local
            .record_models_effect_result(lease, begun, outcome, self.clock.now())
        {
            Ok(lease) => {
                self.lease = Some(lease);
                Ok(())
            }
            Err(error) => Err(result_unconfirmed(error, self.intent_id.clone())),
        }
    }
}

fn preparation_stop(error: ChainPostCloseError, context_intent_id: &str) -> anyhow::Error {
    let intent_id = match &error {
        ChainPostCloseError::RunConflict { intent_id }
        | ChainPostCloseError::LeaseHeld { intent_id }
        | ChainPostCloseError::StaleLease { intent_id }
        | ChainPostCloseError::LeaseExpired { intent_id }
        | ChainPostCloseError::IncompleteEffect { intent_id } => intent_id.clone(),
        _ => context_intent_id.to_owned(),
    };
    let stop = match &error {
        ChainPostCloseError::IncompleteEffect { .. } => {
            PreparationStop::IncompleteOnReopen { intent_id }
        }
        _ => PreparationStop::AuthorityRejected { intent_id },
    };
    anyhow::Error::new(error).context(stop)
}

fn result_unconfirmed(error: ChainPostCloseError, intent_id: String) -> anyhow::Error {
    anyhow::Error::new(error).context(PreparationStop::ResultUnconfirmed { intent_id })
}

impl ChainPostClose<'_> {
    pub(crate) fn install_schema(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::install(&mut self.store.connection)
    }

    pub(crate) fn verify_schema(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::verify_current(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v1_to_v2(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v2(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v2_to_v3(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v3(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v3_to_v4(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v4(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v4_to_v5(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v5(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v5_to_v6(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v6(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v6_to_v7(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v7(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v8_to_v9(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v9(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v9_to_v10(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v10(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v10_to_v11(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v11(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v11_to_v12(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v12(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v12_to_v13(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v13(&mut self.store.connection)
    }

    pub(crate) fn migrate_schema_v7_to_v8(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::migrate_v8(&mut self.store.connection)
    }

    #[cfg(test)]
    pub(crate) fn verify_schema_v1_reader(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::verify(&mut self.store.connection)
    }

    #[cfg(test)]
    pub(crate) fn verify_schema_v2_reader(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::verify_v2_reader(&mut self.store.connection)
    }

    #[cfg(test)]
    pub(crate) fn verify_schema_v3_reader(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::verify_v3_reader(&mut self.store.connection)
    }

    #[cfg(test)]
    pub(crate) fn verify_schema_v10_reader(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::verify_v10_reader(&mut self.store.connection)
    }

    #[cfg(test)]
    pub(crate) fn verify_schema_v11_reader(
        &mut self,
    ) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
        schema::verify_v11_reader(&mut self.store.connection)
    }
}
