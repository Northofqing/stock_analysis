//! Macro facts on the BusinessIntentStore's owned connection; no network here.
use chrono::{DateTime, FixedOffset, TimeZone as _, Utc};
use rusqlite::{params, Transaction, TransactionBehavior};

use super::macro_codec::{self as codec, require, NewsResult, Plan, RawResult, Result};
use super::{
    check_lease, dragon_tiger, inspect_run_and_macro_on, inspect_run_and_macro_scoped,
    inspect_run_on, schema, storage, ChainPostCloseError, LocalChainPostClose, RunLease,
    RunRecovery,
};
use crate::data_gateway::review::map_gateway_audit_record;
use crate::data_gateway::{GatewayBatch, GatewayError, GlobalNewsProvider, GlobalNewsRecord};
use crate::database::data_acquisition_audit::{
    append_acquisition_in_transaction, verify_acquisition_receipt_in_transaction,
    DataAcquisitionAuditReceipt,
};
use crate::grpc_client::client::external_control_attempt::{
    AuthorizedCapabilitiesAttempt, AuthorizedHealthAttempt, ExternalControlCompletion,
    ExternalControlKind, ExternalControlRequestMaterial,
};
use crate::grpc_client::client::macro_attempt::{
    AuthorizedMacroAttempt, AuthorizedPreparedMacroRequest, ExternalMacroAttemptCompletion,
    MacroAttemptCompletion, MacroContinuation, MacroQueryIdentity,
};
use crate::grpc_client::external_pb::magic::market::v1::{
    CapabilitiesResponse, HealthResponse, Operation as ExternalOperation,
};
use crate::grpc_client::provider_attempts::{ExternalProviderCatalog, ProviderAttempts};
use crate::grpc_client::retry::RetryDecision;
use crate::monitor::push_job::{raw_digest, IntentId, Sha256Digest, UtcMicros};
use crate::search_service::service::MacroWebSnapshot;

pub(super) const TABLES: [&str; 8] = [
    "chain_post_close_macro_plans",
    "chain_post_close_macro_request_plans",
    "chain_post_close_macro_readiness_episode_plans",
    "chain_post_close_macro_control_attempt_begins",
    "chain_post_close_macro_control_attempt_results",
    "chain_post_close_macro_attempt_begins",
    "chain_post_close_macro_attempt_results",
    "chain_post_close_macro_source_finals",
];
const CAPABILITY: &str = "GlobalNews-Eastmoney";
const REQUEST_HASH: &str = "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c";

pub(crate) struct MacroAttemptRecovery {
    pub(super) query: crate::search_service::macro_news::runner::QueryKey,
    pub(super) request: Vec<u8>,
    pub(super) request_id: String,
    pub(super) ordinal: u32,
    pub(super) begin: u64,
    pub(super) result: Option<u64>,
    pub(super) readiness_result: Option<u64>,
    pub(super) material: Option<codec::RecoveredResult>,
    pub(super) retry_not_before: Option<i64>,
}

pub(crate) enum MacroRecoveredTrailer<'a> {
    Absent,
    Bytes(&'a [u8]),
    Malformed,
}

pub(crate) enum MacroRecoveredWire<'a> {
    ConnectUnavailable,
    LocalWireFailure,
    Response(&'a [u8]),
    Status {
        code: i32,
        details: &'a [u8],
        trailer: MacroRecoveredTrailer<'a>,
    },
}

pub(crate) struct MacroAttemptResultRecovery<'a> {
    pub(crate) wire: MacroRecoveredWire<'a>,
    pub(crate) diagnostic: Option<&'a str>,
    pub(crate) retry_decision: RetryDecision,
    pub(crate) continuation: MacroContinuation,
    provider_attempts: Option<&'a ProviderAttempts>,
}

impl<'a> MacroAttemptResultRecovery<'a> {
    pub(crate) fn provider_attempts(&self) -> Option<&'a ProviderAttempts> {
        self.provider_attempts
    }
}

impl MacroAttemptRecovery {
    pub(crate) fn query_key(&self) -> crate::search_service::macro_news::runner::QueryKey {
        self.query
    }
    pub(crate) fn request_bytes(&self) -> &[u8] {
        &self.request
    }
    pub(crate) fn attempt_ordinal(&self) -> u32 {
        self.ordinal
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
    pub(crate) fn readiness_result_version(&self) -> Option<u64> {
        self.readiness_result
    }
    pub(crate) fn begin_version(&self) -> u64 {
        self.begin
    }
    pub(crate) fn result_version(&self) -> Option<u64> {
        self.result
    }
    pub(crate) fn response_bytes(&self) -> Option<&[u8]> {
        match &self.material.as_ref()?.wire {
            codec::RecoveredWire::Response(response) => Some(response),
            codec::RecoveredWire::ConnectUnavailable
            | codec::RecoveredWire::LocalWireFailure
            | codec::RecoveredWire::Status { .. } => None,
        }
    }
    pub(crate) fn continuation(&self) -> Option<MacroContinuation> {
        self.material.as_ref().map(|material| material.continuation)
    }
    pub(crate) fn result_material(&self) -> Option<MacroAttemptResultRecovery<'_>> {
        let material = self.material.as_ref()?;
        let wire = match &material.wire {
            codec::RecoveredWire::ConnectUnavailable => MacroRecoveredWire::ConnectUnavailable,
            codec::RecoveredWire::LocalWireFailure => MacroRecoveredWire::LocalWireFailure,
            codec::RecoveredWire::Response(response) => {
                MacroRecoveredWire::Response(response.as_slice())
            }
            codec::RecoveredWire::Status {
                code,
                details,
                trailer,
            } => MacroRecoveredWire::Status {
                code: *code,
                details: details.as_slice(),
                trailer: match trailer {
                    codec::Trailer::Absent => MacroRecoveredTrailer::Absent,
                    codec::Trailer::Bytes(bytes) => MacroRecoveredTrailer::Bytes(bytes.as_slice()),
                    codec::Trailer::Malformed => MacroRecoveredTrailer::Malformed,
                },
            },
        };
        Some(MacroAttemptResultRecovery {
            wire,
            diagnostic: material.diagnostic.as_deref(),
            retry_decision: material.retry_decision,
            continuation: material.continuation,
            provider_attempts: material.provider_attempts.as_ref(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacroControlOutcome {
    Ready,
    Rejected,
}

pub(crate) struct MacroControlRecovery {
    pub(super) request: codec::ControlRequest,
    pub(super) begin: Option<u64>,
    pub(super) result: Option<u64>,
    pub(super) outcome: Option<MacroControlOutcome>,
    pub(super) response: Option<Vec<u8>>,
}

impl MacroControlRecovery {
    pub(crate) fn kind(&self) -> ExternalControlKind {
        self.request.kind()
    }
    pub(crate) fn request_bytes(&self) -> &[u8] {
        self.request.request_bytes()
    }
    pub(crate) fn request_id(&self) -> &str {
        self.request.request_id()
    }
    pub(crate) fn begin_version(&self) -> Option<u64> {
        self.begin
    }
    pub(crate) fn result_version(&self) -> Option<u64> {
        self.result
    }
    pub(crate) fn outcome(&self) -> Option<MacroControlOutcome> {
        self.outcome
    }
    pub(crate) fn response_bytes(&self) -> Option<&[u8]> {
        self.response.as_deref()
    }
    pub(super) fn request_material(&self) -> ExternalControlRequestMaterial {
        self.request.material()
    }
}

pub(crate) struct MacroReadinessEpisodeRecovery {
    pub(super) plan: codec::ReadinessEpisodePlan,
    pub(super) plan_version: u64,
    pub(super) initiating_source: MacroQueryIdentity,
    pub(super) controls: Vec<MacroControlRecovery>,
    pub(super) ready_result: Option<u64>,
    provider_catalog: Option<ExternalProviderCatalog>,
}

impl MacroReadinessEpisodeRecovery {
    pub(crate) fn episode_ordinal(&self) -> u32 {
        self.plan.episode_ordinal()
    }
    pub(crate) fn initiating_source(&self) -> &MacroQueryIdentity {
        // v11 fixes episode 1 to Gateway/GlobalNews-Eastmoney.
        &self.initiating_source
    }
    pub(crate) fn controls(&self) -> &[MacroControlRecovery] {
        &self.controls
    }
    pub(crate) fn ready_result_version(&self) -> Option<u64> {
        self.ready_result
    }
}

pub(super) fn historical_provider_catalog(
    episodes: &[MacroReadinessEpisodeRecovery],
    readiness_result: Option<u64>,
) -> Option<&ExternalProviderCatalog> {
    let readiness_result = readiness_result?;
    episodes
        .iter()
        .find(|episode| episode.ready_result == Some(readiness_result))?
        .provider_catalog
        .as_ref()
}

pub(crate) struct MacroNewsRecovery {
    pub(super) result: NewsResult,
    pub(super) bytes: Vec<u8>,
    pub(super) receipt: DataAcquisitionAuditReceipt,
    pub(super) policy: (u32, u64, u64, u64),
    pub(super) profile: String,
    pub(super) authority: Option<String>,
}
impl MacroNewsRecovery {
    pub(crate) fn is_complete(&self) -> bool {
        true
    }
    pub(crate) fn profile(&self) -> &str {
        &self.profile
    }
    pub(crate) fn acquisition_authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }
    pub(crate) fn retry_policy(&self) -> (u32, u64, u64, u64) {
        self.policy
    }
    pub(crate) fn batch(&self) -> Option<&GatewayBatch<GlobalNewsRecord>> {
        self.result.as_ref().ok()
    }
    pub(crate) fn error(&self) -> Option<&GatewayError> {
        self.result.as_ref().err()
    }
    pub(crate) fn final_bytes(&self) -> Option<&[u8]> {
        Some(&self.bytes)
    }
    pub(crate) fn audit_receipt(&self) -> Option<&DataAcquisitionAuditReceipt> {
        Some(&self.receipt)
    }
}

pub(crate) struct MacroRecovery {
    pub(super) full: Option<super::macro_recovery::FullRecovery>,
    pub(super) plan: Plan,
    pub(super) plan_bytes: Vec<u8>,
    pub(super) plan_version: u64,
    pub(super) request_plan_version: u64,
    pub(super) attempts: Vec<MacroAttemptRecovery>,
    pub(super) readiness_episodes: Vec<MacroReadinessEpisodeRecovery>,
    pub(super) source: Option<MacroNewsRecovery>,
}
impl MacroRecovery {
    pub(crate) fn is_complete(&self) -> bool {
        self.full.as_ref().is_some_and(|full| full.final_.is_some())
    }
    pub(crate) fn has_unconfirmed_effect(&self) -> bool {
        self.attempts.iter().any(|a| a.result.is_none())
            || self
                .full
                .as_ref()
                .is_some_and(|full| full.begin.is_some() && full.final_.is_none())
            || self.readiness_episodes.iter().any(|episode| {
                episode
                    .controls
                    .iter()
                    .any(|control| control.begin.is_some() && control.result.is_none())
            })
    }
    pub(crate) fn plan(&self) -> &Plan {
        &self.plan
    }
    pub(crate) fn plan_bytes(&self) -> &[u8] {
        &self.plan_bytes
    }
    pub(crate) fn plan_version(&self) -> u64 {
        self.plan_version
    }
    pub(crate) fn parent_final_bytes(&self) -> &[u8] {
        &self.plan.parent_bytes
    }
    pub(crate) fn attempts(&self) -> &[MacroAttemptRecovery] {
        &self.attempts
    }
    pub(crate) fn readiness_episodes(&self) -> &[MacroReadinessEpisodeRecovery] {
        &self.readiness_episodes
    }
    pub(crate) fn pending_source_identities(&self) -> &[MacroQueryIdentity] {
        if let Some(full) = &self.full {
            return &full.pending_sources;
        }
        &self.plan.source_identities()[usize::from(self.source.is_some())..]
    }
    pub(crate) fn pending_research_queries(&self) -> &[String] {
        if let Some(full) = &self.full {
            return &full.pending_research;
        }
        self.plan.research_queries()
    }
    pub(crate) fn global_news(&self, provider: GlobalNewsProvider) -> Option<&MacroNewsRecovery> {
        if let Some(full) = &self.full {
            return full
                .news
                .iter()
                .find(|(admitted, _)| *admitted == provider)
                .map(|(_, news)| news);
        }
        if provider == GlobalNewsProvider::Eastmoney {
            self.source.as_ref()
        } else {
            None
        }
    }
}

pub(super) struct FactExpectedDigests<'run> {
    run: &'run RunRecovery,
    context: Option<Sha256Digest>,
    input: Option<Sha256Digest>,
}
impl<'run> FactExpectedDigests<'run> {
    pub(super) fn new(run: &'run RunRecovery) -> Self {
        Self {
            run,
            context: None,
            input: None,
        }
    }
    fn context(&mut self) -> &str {
        let run = self.run;
        self.context
            .get_or_insert_with(|| run.context.canonical_sha256())
            .as_str()
    }
    fn input(&mut self) -> Result<&str> {
        let run = self.run;
        if self.input.is_none() {
            self.input = Some(raw_digest(&run.input.encode()?));
        }
        Ok(self
            .input
            .as_ref()
            .expect("input digest set after successful encoding")
            .as_str())
    }
}

#[derive(Clone)]
pub(super) struct Fact {
    pub(super) run_id: String,
    pub(super) context: String,
    pub(super) input: String,
    pub(super) owner: String,
    pub(super) generation: u64,
    pub(super) prior: u64,
    pub(super) version: u64,
    pub(super) time: i64,
    pub(super) bytes: Vec<u8>,
    pub(super) length: usize,
    pub(super) digest: String,
}
impl Fact {
    fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            run_id: row.get(0)?,
            context: row.get(1)?,
            input: row.get(2)?,
            owner: row.get(3)?,
            generation: row.get(4)?,
            prior: row.get(5)?,
            version: row.get(6)?,
            time: row.get(7)?,
            bytes: row.get(8)?,
            length: row.get(9)?,
            digest: row.get(10)?,
        })
    }
    pub(super) fn validate(&self, run: &RunRecovery) -> Result<()> {
        let mut expected = FactExpectedDigests::new(run);
        self.validate_with_expected(&mut expected)
    }
    pub(super) fn validate_with_expected(
        &self,
        expected: &mut FactExpectedDigests<'_>,
    ) -> Result<()> {
        let run = expected.run;
        require(
            self.run_id == run.context.run_id().as_str()
                && self.context == expected.context()
                && self.input == expected.input()?
                && self.generation > 0
                && self.generation <= run.generation
                && !self.owner.is_empty()
                && (self.generation < run.generation || self.owner == run.owner)
                && self.prior.checked_add(1) == Some(self.version)
                && self.version <= run.head
                && self.time >= 0
                && self.time <= run.updated_at
                && !self.bytes.is_empty()
                && self.bytes.len() == self.length
                && raw_digest(&self.bytes).as_str() == self.digest,
        )
    }
}

pub(super) fn facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    table: &str,
) -> Result<Vec<Fact>> {
    require(TABLES.contains(&table) || super::macro_recovery::TABLES.contains(&table))?;
    let mut statement=transaction.prepare(&format!("SELECT run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256 FROM {table} WHERE intent_id=?1 ORDER BY run_version"))
        .map_err(|_|storage("macro facts"))?;
    let rows = statement
        .query_map([intent.as_str()], Fact::row)
        .map_err(|_| storage("macro facts"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("macro facts"))?;
    Ok(rows)
}

pub(super) fn audit_time(time: i64) -> Result<String> {
    Ok(Utc
        .timestamp_micros(time)
        .single()
        .ok_or(ChainPostCloseError::SchemaRejected)?
        .to_rfc3339())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Begin {
    pub(super) version: u32,
    pub(super) ordinal: u32,
    pub(super) plan_sha256: String,
    pub(super) request_sha256: String,
    pub(super) previous_result: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ControlBegin {
    pub(super) version: u32,
    pub(super) episode_ordinal: u32,
    pub(super) control_ordinal: u32,
    pub(super) kind: String,
    pub(super) episode_sha256: String,
    pub(super) request_sha256: String,
    pub(super) health_result_version: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceFinal {
    pub(super) version: u32,
    pub(super) phase: String,
    pub(super) item_ordinal: u32,
    pub(super) cause_kind: String,
    pub(super) cause_version: u64,
    pub(super) native_sha256: String,
}

pub(super) fn load_on(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<Option<MacroRecovery>> {
    if schema::runtime_layout_version(transaction)? >= 12 {
        return super::macro_recovery::load_on(transaction, intent, run, None);
    }
    load_on_with_dragon_validation(
        transaction,
        intent,
        run,
        DragonTigerValidation::Independent,
        false,
    )
}

pub(super) fn load_on_after_dragon_validation<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    validated: &dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>,
) -> Result<Option<MacroRecovery>> {
    load_on_after_dragon_validation_scoped(transaction, intent, run, validated, None)
}

pub(super) fn load_on_after_dragon_validation_scoped<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    validated: &dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<Option<MacroRecovery>> {
    if let Some(proof) = proof {
        proof.check(transaction)?;
        return super::macro_recovery::load_on(transaction, intent, run, Some(validated));
    }
    if schema::runtime_layout_version(transaction)? >= 12 {
        return super::macro_recovery::load_on(transaction, intent, run, Some(validated));
    }
    load_on_with_dragon_validation(
        transaction,
        intent,
        run,
        DragonTigerValidation::Reused(validated),
        false,
    )
}

enum DragonTigerValidation<'validated, 'transaction, 'connection, 'run> {
    Independent,
    Reused(&'validated dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>),
}

// Only the v12 reader may request this historical subset. The original v11
// entry remains exhaustive, and the v12 caller validates every excluded row.
pub(super) fn load_legacy_subset<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    validated: Option<&dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>>,
) -> Result<Option<MacroRecovery>> {
    schema::verify_parent_layout_v12(transaction)?;
    let validation = match validated {
        Some(validated) => DragonTigerValidation::Reused(validated),
        None => DragonTigerValidation::Independent,
    };
    load_on_with_dragon_validation(transaction, intent, run, validation, true)
}

fn compatibility_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    table: &str,
    historical_subset: bool,
) -> Result<Vec<Fact>> {
    let rows = facts(transaction, intent, table)?;
    if !historical_subset {
        return Ok(rows);
    }
    rows.into_iter()
        .filter_map(|fact| {
            let value = match serde_json::from_slice::<serde_json::Value>(&fact.bytes) {
                Ok(value) => value,
                Err(_) => return Some(Err(ChainPostCloseError::SchemaRejected)),
            };
            (value.get("version").and_then(serde_json::Value::as_u64) != Some(2))
                .then_some(Ok(fact))
        })
        .collect()
}

fn compatibility_results(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    historical_begins: &[Fact],
    historical_subset: bool,
) -> Result<Vec<Fact>> {
    let results = facts(transaction, intent, TABLES[6])?;
    if !historical_subset {
        return Ok(results);
    }
    let mut statement = transaction
        .prepare(
            "SELECT run_version,begin_version FROM chain_post_close_macro_attempt_results \
             WHERE intent_id=?1 ORDER BY run_version",
        )
        .map_err(|_| storage("macro result compatibility links"))?;
    let links = statement
        .query_map([intent.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|_| storage("macro result compatibility links"))?
        .collect::<rusqlite::Result<Vec<(u64, u64)>>>()
        .map_err(|_| storage("macro result compatibility links"))?;
    require(results.len() == links.len())?;
    results
        .into_iter()
        .zip(links)
        .filter_map(|(result, (result_version, begin_version))| {
            if result.version != result_version {
                return Some(Err(ChainPostCloseError::SchemaRejected));
            }
            historical_begins
                .iter()
                .any(|begin| begin.version == begin_version)
                .then_some(Ok(result))
        })
        .collect()
}

fn load_on_with_dragon_validation<'validated, 'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    dragon_tiger_validation: DragonTigerValidation<'validated, 'transaction, 'connection, 'run>,
    legacy_subset: bool,
) -> Result<Option<MacroRecovery>> {
    let plans = facts(transaction, intent, TABLES[0])?;
    let request_plans = compatibility_facts(transaction, intent, TABLES[1], legacy_subset)?;
    let episode_plans = facts(transaction, intent, TABLES[2])?;
    let control_begins = facts(transaction, intent, TABLES[3])?;
    let control_results = facts(transaction, intent, TABLES[4])?;
    let begins = compatibility_facts(transaction, intent, TABLES[5], legacy_subset)?;
    let results =
        compatibility_results(transaction, intent, begins.as_slice(), legacy_subset)?;
    let finals = facts(transaction, intent, TABLES[7])?;
    let Some(fact) = plans.first() else {
        require(
            request_plans.is_empty()
                && episode_plans.is_empty()
                && control_begins.is_empty()
                && control_results.is_empty()
                && begins.is_empty()
                && results.is_empty()
                && finals.is_empty(),
        )?;
        return Ok(None);
    };
    require(
        plans.len() == 1
            && request_plans.len() == 1
            && (control_results.len() == control_begins.len()
                || control_results.len() + 1 == control_begins.len())
            && (results.len() == begins.len() || results.len() + 1 == begins.len())
            && finals.len() <= 1,
    )?;
    for fact in plans
        .iter()
        .chain(&request_plans)
        .chain(&episode_plans)
        .chain(&control_begins)
        .chain(&control_results)
        .chain(&begins)
        .chain(&results)
        .chain(&finals)
    {
        fact.validate(run)?;
    }
    let parent = match dragon_tiger_validation {
        DragonTigerValidation::Independent => dragon_tiger::macro_parent(transaction, intent, run)?,
        DragonTigerValidation::Reused(validated) => {
            dragon_tiger::macro_parent_from_validated(transaction, intent, run, validated)?
        }
    };
    let mut plan: Plan = codec::decode(&fact.bytes)?;
    plan.validate(&parent)?;
    let (parent_version,parent_sha,started,deadline,request_sha):(u64,String,i64,i64,String)=transaction.query_row(
        "SELECT parent_version,parent_sha256,started_at,deadline_at,request_sha256 FROM chain_post_close_macro_plans WHERE intent_id=?1",
        [intent.as_str()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))
        .map_err(|_|storage("macro plan"))?;
    require(
        parent_version == plan.parent_version
            && parent_sha == plan.parent_digest
            && started == plan.started
            && deadline == plan.deadline
            && request_sha == raw_digest(&plan.request.bytes).as_str()
            && fact.prior >= parent_version
            && fact.time >= started
            && fact.time < deadline,
    )?;
    let request_fact = &request_plans[0];
    let request: codec::Request = codec::decode(&request_fact.bytes)?;
    request.validate()?;
    type RequestExtra = (
        String,
        u32,
        u32,
        u64,
        String,
        String,
        Option<String>,
        String,
    );
    let request_extra: RequestExtra = transaction
        .query_row(
            "SELECT phase,item_ordinal,candidate_ordinal,plan_version,request_sha256,profile,acquisition_authority,endpoint FROM chain_post_close_macro_request_plans WHERE intent_id=?1",
            [intent.as_str()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        )
        .map_err(|_| storage("macro request plan"))?;
    require(
        request_extra.0 == "Gateway"
            && request_extra.1 == 1
            && request_extra.2 == 1
            && request_extra.3 == fact.version
            && request_extra.4 == raw_digest(&request.bytes).as_str()
            && request_extra.5 == request.profile
            && request_extra.6 == request.authority
            && request_extra.7 == plan.endpoint
            && request_fact.prior == fact.version
            && request_fact.owner == fact.owner
            && request_fact.generation == fact.generation
            && request_fact.time == fact.time
            && request_fact.time < deadline
            && codec::encode(&request)? == codec::encode(&plan.request)?,
    )?;

    let (readiness_episodes, rejected_control, ready_result_time) = recover_readiness(
        transaction,
        intent,
        &plan,
        fact,
        request_fact,
        &episode_plans,
        &control_begins,
        &control_results,
        false,
    )?;

    let mut attempts = Vec::<MacroAttemptRecovery>::new();
    let readiness_result = readiness_episodes
        .first()
        .and_then(MacroReadinessEpisodeRecovery::ready_result_version);
    let mut terminal_data: Option<(u64, NewsResult)> = None;
    for (index, begin) in begins.iter().enumerate() {
        let ordinal = u32::try_from(index + 1).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let decoded: Begin = codec::decode(&begin.bytes)?;
        type DataBeginExtra = (String, u32, u32, u32, u64, String, Option<u64>, Option<u64>);
        let extra:DataBeginExtra=transaction.query_row(
            "SELECT phase,item_ordinal,candidate_ordinal,attempt_ordinal,request_plan_version,request_sha256,readiness_result_version,previous_result_version FROM chain_post_close_macro_attempt_begins WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(),begin.version],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)))
            .map_err(|_|storage("macro begin"))?;
        let last = attempts.last();
        require(
            terminal_data.is_none()
                && extra.0 == "Gateway"
                && extra.1 == 1
                && extra.2 == 1
                && extra.3 == ordinal
                && ordinal <= plan.request.policy.0
                && extra.4 == request_fact.version
                && extra.5 == request_sha
                && if plan.profile() == crate::grpc_client::client::ContractProfile::ExternalV1 {
                    extra.6.is_some()
                        && extra.6 == readiness_result
                        && readiness_result.is_some_and(|version| {
                            version < begin.version && version <= begin.prior
                        })
                        && ready_result_time.is_some_and(|time| time <= begin.time)
                } else {
                    extra.6.is_none()
                }
                && begin.prior >= request_fact.version
                && begin.time >= fact.time
                && begin.time < deadline
                && decoded.version == 1
                && decoded.ordinal == ordinal
                && decoded.plan_sha256 == fact.digest
                && decoded.request_sha256 == request_sha
                && extra.7 == last.and_then(|a| a.result)
                && decoded.previous_result == extra.7
                && last.is_none_or(|a| {
                    a.result.is_some()
                        && matches!(a.continuation(), Some(MacroContinuation::Retry { .. }))
                        && a.retry_not_before.is_some_and(|due| due <= begin.time)
                }),
        )?;
        let provider_catalog =
            historical_provider_catalog(&readiness_episodes, extra.6);
        let mut attempt = MacroAttemptRecovery {
            query: crate::search_service::macro_news::runner::QueryKey::Gateway(1),
            request: plan.request.bytes.clone(),
            request_id: plan.request.id.clone(),
            ordinal,
            begin: begin.version,
            result: None,
            readiness_result: extra.6,
            material: None,
            retry_not_before: None,
        };
        if let Some(result) = results.get(index) {
            type DataResultExtra = (String, u32, u32, u32, u64, String, String, Option<i64>);
            let result_extra:DataResultExtra=transaction.query_row("SELECT phase,item_ordinal,candidate_ordinal,attempt_ordinal,begin_version,request_sha256,continuation,retry_not_before FROM chain_post_close_macro_attempt_results WHERE intent_id=?1 AND run_version=?2",
                params![intent.as_str(),result.version],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)))
                .map_err(|_|storage("macro result"))?;
            require(
                result_extra.0 == "Gateway"
                    && result_extra.1 == 1
                    && result_extra.2 == 1
                    && result_extra.3 == ordinal
                    && result_extra.4 == begin.version
                    && result_extra.5 == request_sha
                    && result.prior >= begin.version
                    && result.time >= begin.time
                    && result.time < deadline
                    && result.owner == begin.owner
                    && result.generation == begin.generation,
            )?;
            let raw: RawResult = codec::decode(&result.bytes)?;
            let (gateway, retry_decision, provider_attempts) =
                raw.project(&plan.request, ordinal, provider_catalog)?;
            match raw.continuation() {
                MacroContinuation::Retry { backoff_ms } => {
                    let delay = i64::try_from(backoff_ms)
                        .ok()
                        .and_then(|v| v.checked_mul(1000))
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    require(
                        result_extra.6 == "Retry"
                            && result_extra.7 == result.time.checked_add(delay),
                    )?;
                }
                MacroContinuation::Terminal => {
                    require(result_extra.6 == "Terminal" && result_extra.7.is_none())?;
                    terminal_data = Some((result.version, gateway));
                }
            }
            attempt.result = Some(result.version);
            attempt.material = Some(raw.into_recovered(retry_decision, provider_attempts)?);
            attempt.retry_not_before = result_extra.7;
        }
        attempts.push(attempt);
    }

    let terminal_cause_count =
        usize::from(terminal_data.is_some()) + usize::from(rejected_control.is_some());
    require(finals.len() == terminal_cause_count)?;
    let source = if let Some(final_fact) = finals.first() {
        type FinalExtra = (
            String,
            u32,
            String,
            u64,
            Vec<u8>,
            usize,
            String,
            i64,
            String,
            Option<String>,
            String,
        );
        let extra:FinalExtra=transaction.query_row("SELECT phase,item_ordinal,cause_kind,cause_version,native_bytes,native_length,native_sha256,audit_id,audit_record_hash,previous_outcome,current_outcome FROM chain_post_close_macro_source_finals WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=1",
            [intent.as_str()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?)))
            .map_err(|_|storage("macro source final"))?;
        let (gateway, cause_fact) = match (terminal_data, rejected_control) {
            (Some((version, gateway)), None) => {
                require(extra.2 == "DataResult" && extra.3 == version)?;
                let cause = results
                    .iter()
                    .find(|result| result.version == version)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                (gateway, cause)
            }
            (None, Some((version, error))) => {
                require(extra.2 == "ExternalControlResult" && extra.3 == version)?;
                let cause = control_results
                    .iter()
                    .find(|result| result.version == version)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                (Err(error), cause)
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        let decoded: SourceFinal = codec::decode(&final_fact.bytes)?;
        require(
            extra.0 == "Gateway"
                && extra.1 == 1
                && final_fact.prior == extra.3
                && final_fact.version
                    == extra
                        .3
                        .checked_add(1)
                        .ok_or(ChainPostCloseError::SchemaRejected)?
                && final_fact.owner == cause_fact.owner
                && final_fact.generation == cause_fact.generation
                && final_fact.time == cause_fact.time
                && final_fact.time < deadline
                && extra.4.len() == extra.5
                && raw_digest(&extra.4).as_str() == extra.6
                && extra.4 == codec::native_bytes(&gateway)?
                && decoded.version == 1
                && decoded.phase == extra.0
                && decoded.item_ordinal == extra.1
                && decoded.cause_kind == extra.2
                && decoded.cause_version == extra.3
                && decoded.native_sha256 == extra.6,
        )?;
        let receipt = DataAcquisitionAuditReceipt {
            audit_id: extra.7,
            record_hash: extra.8,
            previous_outcome: extra.9,
            current_outcome: extra.10,
        };
        let audit = map_gateway_audit_record(
            CAPABILITY,
            GlobalNewsProvider::Eastmoney.provider_id(),
            REQUEST_HASH,
            &gateway,
            &audit_time(final_fact.time)?,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_acquisition_receipt_in_transaction(
            transaction,
            &receipt,
            &audit.borrowed(CAPABILITY),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        Some(MacroNewsRecovery {
            result: gateway,
            bytes: extra.4,
            receipt,
            policy: request.policy,
            profile: request.profile.clone(),
            authority: request.authority.clone(),
        })
    } else {
        None
    };
    Ok(Some(MacroRecovery {
        full: None,
        plan,
        plan_bytes: fact.bytes.clone(),
        plan_version: fact.version,
        request_plan_version: request_fact.version,
        attempts,
        readiness_episodes,
        source,
    }))
}

/// Shared control-envelope validator; callers validate the complete catalog and
/// fact headers first. The full-layout caller has already checked all four
/// preregistered news requests, so its episode need not adjoin request one.
pub(super) fn recover_readiness(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    plan: &Plan,
    fact: &Fact,
    request_fact: &Fact,
    episode_plans: &[Fact],
    control_begins: &[Fact],
    control_results: &[Fact],
    full_layout: bool,
) -> Result<(
    Vec<MacroReadinessEpisodeRecovery>,
    Option<(u64, GatewayError)>,
    Option<i64>,
)> {
    let deadline = plan.deadline;
    let mut readiness_episodes = Vec::new();
    let mut rejected_control: Option<(u64, GatewayError)> = None;
    let mut ready_result_time = None;
    if plan.profile() == crate::grpc_client::client::ContractProfile::ExternalV1 {
        require(
            episode_plans.len() == 1
                && control_begins.len() <= 2
                && control_results.len() <= control_begins.len()
                && control_begins.len() - control_results.len() <= 1,
        )?;
        let episode_fact = &episode_plans[0];
        let episode_plan: codec::ReadinessEpisodePlan = codec::decode(&episode_fact.bytes)?;
        episode_plan.validate()?;
        type EpisodeExtra = (
            u32,
            u64,
            u64,
            String,
            u32,
            u32,
            i32,
            String,
            String,
            String,
            String,
        );
        let episode_extra: EpisodeExtra = transaction
            .query_row(
                "SELECT episode_ordinal,plan_version,request_plan_version,phase,item_ordinal,candidate_ordinal,required_operation,endpoint,acquisition_authority,health_request_sha256,capabilities_request_sha256 FROM chain_post_close_macro_readiness_episode_plans WHERE intent_id=?1",
                [intent.as_str()],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?)),
            )
            .map_err(|_| storage("macro readiness plan"))?;
        let planned_controls = episode_plan.controls();
        require(
            episode_extra.0 == 1
                && episode_extra.1 == fact.version
                && episode_extra.2 == request_fact.version
                && episode_extra.3 == "Gateway"
                && episode_extra.4 == 1
                && episode_extra.5 == 1
                && episode_extra.6 == ExternalOperation::GlobalNews as i32
                && episode_extra.7 == plan.endpoint
                && Some(episode_extra.8.as_str()) == plan.acquisition_authority()
                && planned_controls
                    .iter()
                    .all(|control| control.endpoint() == plan.endpoint)
                && planned_controls
                    .iter()
                    .all(|control| Some(control.authority()) == plan.acquisition_authority())
                && episode_extra.9 == raw_digest(planned_controls[0].request_bytes()).as_str()
                && episode_extra.10 == raw_digest(planned_controls[1].request_bytes()).as_str()
                && (if full_layout {
                    episode_fact.prior >= request_fact.version
                } else {
                    episode_fact.prior == request_fact.version
                })
                && episode_fact.owner == request_fact.owner
                && episode_fact.generation == request_fact.generation
                && episode_fact.time == request_fact.time
                && episode_fact.time < deadline,
        )?;
        let mut controls = Vec::new();
        let mut health_ready_result = None;
        let mut health_ready_time = None;
        let mut ready_result = None;
        let mut provider_catalog = None;
        for (index, planned) in planned_controls.into_iter().enumerate() {
            let ordinal =
                u32::try_from(index + 1).map_err(|_| ChainPostCloseError::SchemaRejected)?;
            let mut recovered = MacroControlRecovery {
                request: planned.clone(),
                begin: None,
                result: None,
                outcome: None,
                response: None,
            };
            if let Some(begin) = control_begins.get(index) {
                let decoded: ControlBegin = codec::decode(&begin.bytes)?;
                type BeginExtra = (u32, u32, String, u64, String, Option<u64>);
                let extra: BeginExtra = transaction
                    .query_row(
                        "SELECT episode_ordinal,control_ordinal,kind,episode_plan_version,request_sha256,health_result_version FROM chain_post_close_macro_control_attempt_begins WHERE intent_id=?1 AND run_version=?2",
                        params![intent.as_str(), begin.version],
                        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
                    )
                    .map_err(|_| storage("macro control begin"))?;
                let kind = if ordinal == 1 {
                    "Health"
                } else {
                    "Capabilities"
                };
                require(
                    extra.0 == 1
                        && extra.1 == ordinal
                        && extra.2 == kind
                        && extra.3 == episode_fact.version
                        && extra.4 == raw_digest(planned.request_bytes()).as_str()
                        && if ordinal == 1 {
                            extra.5.is_none()
                        } else {
                            extra.5.is_some()
                                && extra.5 == health_ready_result
                                && health_ready_result.is_some_and(|version| {
                                    version < begin.version && version <= begin.prior
                                })
                                && health_ready_time.is_some_and(|time| time <= begin.time)
                        }
                        && begin.prior >= episode_fact.version
                        && begin.time >= episode_fact.time
                        && begin.time < deadline
                        && decoded.version == 1
                        && decoded.episode_ordinal == 1
                        && decoded.control_ordinal == ordinal
                        && decoded.kind == kind
                        && decoded.episode_sha256 == episode_fact.digest
                        && decoded.request_sha256 == extra.4
                        && decoded.health_result_version == extra.5,
                )?;
                recovered.begin = Some(begin.version);
                if let Some(result) = control_results.get(index) {
                    type ResultExtra = (u32, u32, String, u64, String, String);
                    let result_extra: ResultExtra = transaction
                        .query_row(
                            "SELECT episode_ordinal,control_ordinal,kind,begin_version,request_sha256,outcome FROM chain_post_close_macro_control_attempt_results WHERE intent_id=?1 AND run_version=?2",
                            params![intent.as_str(), result.version],
                            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
                        )
                        .map_err(|_| storage("macro control result"))?;
                    require(
                        result_extra.0 == 1
                            && result_extra.1 == ordinal
                            && result_extra.2 == kind
                            && result_extra.3 == begin.version
                            && result_extra.4 == extra.4
                            && result.prior >= begin.version
                            && result.time >= begin.time
                            && result.time < deadline
                            && result.owner == begin.owner
                            && result.generation == begin.generation,
                    )?;
                    let raw: codec::ControlRawResult = codec::decode(&result.bytes)?;
                    let projected = raw.project(planned)?;
                    let outcome = if projected.is_ok() {
                        MacroControlOutcome::Ready
                    } else {
                        MacroControlOutcome::Rejected
                    };
                    require(
                        result_extra.5
                            == match outcome {
                                MacroControlOutcome::Ready => "Ready",
                                MacroControlOutcome::Rejected => "Rejected",
                            },
                    )?;
                    if outcome == MacroControlOutcome::Ready {
                        if ordinal == 1 {
                            health_ready_result = Some(result.version);
                            health_ready_time = Some(result.time);
                        } else {
                            require(health_ready_result.is_some())?;
                            provider_catalog = Some(raw.validated_provider_catalog(planned)?);
                            ready_result = Some(result.version);
                            ready_result_time = Some(result.time);
                        }
                    } else {
                        require(rejected_control.is_none() && ready_result.is_none())?;
                        rejected_control = Some((
                            result.version,
                            projected.expect_err("validated rejected control"),
                        ));
                    }
                    recovered.result = Some(result.version);
                    recovered.outcome = Some(outcome);
                    recovered.response = raw.response_bytes().map(ToOwned::to_owned);
                }
            }
            controls.push(recovered);
        }
        readiness_episodes.push(MacroReadinessEpisodeRecovery {
            plan: episode_plan,
            plan_version: episode_fact.version,
            initiating_source: codec::first_identity(),
            controls,
            ready_result,
            provider_catalog,
        });
    } else {
        require(
            episode_plans.is_empty() && control_begins.is_empty() && control_results.is_empty(),
        )?;
    }

    Ok((readiness_episodes, rejected_control, ready_result_time))
}

pub(super) fn validate_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<()> {
    load_on(transaction, intent, run).map(|_| ())
}

pub(super) fn advance(
    transaction: &Transaction<'_>,
    lease: &mut RunLease,
    now: UtcMicros,
) -> Result<u64> {
    let previous = lease.head;
    let next = previous
        .checked_add(1)
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let count=transaction.execute("UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 WHERE intent_id=?3 AND run_id=?4 AND lease_owner=?5 AND lease_generation=?6 AND head_version=?7 AND lease_until>?2 AND updated_at<=?2",
        params![next,now.get(),lease.intent_id.as_str(),lease.run_id.as_str(),lease.owner.as_str(),lease.generation,previous])
        .map_err(|_|storage("macro cas"))?;
    if count != 1 {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    lease.head = next;
    Ok(previous)
}

fn admission(
    transaction: &Transaction<'_>,
    lease: &RunLease,
    now: UtcMicros,
) -> Result<(RunRecovery, Option<MacroRecovery>)> {
    admission_scoped(transaction, lease, now, |_, _, _| Ok(()))
        .map(|(run, macro_recovery, _)| (run, macro_recovery))
}

pub(super) fn validate_admission(
    transaction: &Transaction<'_>,
    lease: &RunLease,
    now: UtcMicros,
    run: &RunRecovery,
) -> Result<()> {
    check_lease(transaction, lease, now)?;
    require(
        run.input.encode()? == lease.input.encode()?
            && run.context.run_id() == &lease.run_id
            && now.get() >= run.updated_at,
    )
}

fn admission_scoped<'transaction, 'connection, T>(
    transaction: &'transaction Transaction<'connection>,
    lease: &RunLease,
    now: UtcMicros,
    consume: impl for<'run> FnOnce(
        &'run RunRecovery,
        &Option<MacroRecovery>,
        dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>,
    ) -> Result<T>,
) -> Result<(RunRecovery, Option<MacroRecovery>, T)> {
    schema::verify_runtime_layout_version(transaction, 11)?;
    let (run, recovery, consumed) =
        inspect_run_and_macro_scoped(transaction, &lease.intent_id, |run, recovery, validated| {
            validate_admission(transaction, lease, now, run)?;
            consume(run, recovery, validated)
        })?;
    Ok((
        run,
        recovery,
        consumed.ok_or(ChainPostCloseError::SchemaRejected)?,
    ))
}

pub(super) struct Call {
    intent: String,
    run_id: String,
    context: String,
    input: String,
    ordinal: u32,
    begin: u64,
    request_sha: String,
    owner: String,
    generation: u64,
    readiness_result: Option<u64>,
}

pub(super) struct ControlCall {
    intent: String,
    run_id: String,
    context: String,
    input: String,
    episode_ordinal: u32,
    control_ordinal: u32,
    begin: u64,
    request_sha: String,
    owner: String,
    generation: u64,
}

impl LocalChainPostClose<'_> {
    pub(crate) fn inspect_macro(&mut self, intent: &IntentId) -> Result<MacroRecovery> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("macro inspect"))?;
        require(matches!(
            schema::runtime_layout_version(&transaction)?,
            11 | 12 | 13
        ))?;
        let (_, recovery) = inspect_run_and_macro_on(&transaction, intent)?;
        let recovery = recovery.ok_or(ChainPostCloseError::MacroNotStarted)?;
        transaction
            .commit()
            .map_err(|_| storage("macro inspect commit"))?;
        Ok(recovery)
    }

    pub(super) fn load_macro(
        &mut self,
        lease: &RunLease,
        now: UtcMicros,
    ) -> Result<Option<MacroRecovery>> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("macro load"))?;
        let (_, recovery) = admission(&transaction, lease, now)?;
        transaction
            .commit()
            .map_err(|_| storage("macro load commit"))?;
        Ok(recovery)
    }

    pub(super) fn recover_macro_parent(
        &mut self,
        lease: &RunLease,
        now: UtcMicros,
    ) -> Result<super::dragon_tiger_codec::Projection> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("macro parent"))?;
        require(matches!(
            schema::runtime_layout_version(&transaction)?,
            11 | 12 | 13
        ))?;
        let (_, _, parent) =
            inspect_run_and_macro_scoped(&transaction, &lease.intent_id, |run, _, validated| {
                validate_admission(&transaction, lease, now, run)?;
                dragon_tiger::macro_parent_from_validated(
                    &transaction,
                    &lease.intent_id,
                    run,
                    &validated,
                )
            })?;
        transaction
            .commit()
            .map_err(|_| storage("macro parent commit"))?;
        Ok(parent
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .projection)
    }

    pub(super) fn plan_macro(
        &mut self,
        lease: RunLease,
        started: UtcMicros,
        observation: DateTime<FixedOffset>,
        endpoint: &str,
        attempt: &AuthorizedMacroAttempt,
        web: &MacroWebSnapshot,
        now: UtcMicros,
    ) -> Result<RunLease> {
        self.plan_macro_request(
            lease,
            started,
            observation,
            endpoint,
            codec::Request::capture(attempt)?,
            None,
            web,
            now,
        )
    }

    pub(super) fn plan_macro_request(
        &mut self,
        mut lease: RunLease,
        started: UtcMicros,
        observation: DateTime<FixedOffset>,
        endpoint: &str,
        request: codec::Request,
        episode: Option<codec::ReadinessEpisodePlan>,
        web: &MacroWebSnapshot,
        now: UtcMicros,
    ) -> Result<RunLease> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("macro plan begin"))?;
        let (run, _, parent) =
            admission_scoped(&transaction, &lease, now, |run, recovery, validated| {
                require(
                    recovery.is_none()
                        && request.policy.0 > 0
                        && (request.profile == "ExternalV1") == episode.is_some(),
                )?;
                dragon_tiger::macro_parent_from_validated(
                    &transaction,
                    &lease.intent_id,
                    run,
                    &validated,
                )
            })?;
        let plan = Plan::new(
            &parent,
            started,
            observation,
            endpoint,
            request.clone(),
            web,
        )?;
        require(now.get() < plan.deadline)?;
        let bytes = codec::encode(&plan)?;
        let digest = raw_digest(&bytes);
        let request_sha = raw_digest(&plan.request.bytes);
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute("INSERT INTO chain_post_close_macro_plans(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,parent_version,parent_sha256,started_at,deadline_at,request_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),bytes,bytes.len(),digest.as_str(),parent.version,parent.digest,plan.started,plan.deadline,request_sha.as_str()])
            .map_err(|_|storage("macro plan insert"))?;
        let plan_version = lease.head;
        let request_bytes = codec::encode(&request)?;
        let request_digest = raw_digest(&request_bytes);
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute("INSERT INTO chain_post_close_macro_request_plans(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,phase,item_ordinal,candidate_ordinal,plan_version,request_sha256,profile,acquisition_authority,endpoint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'Gateway',1,1,?13,?14,?15,?16,?17)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),request_bytes,request_bytes.len(),request_digest.as_str(),plan_version,raw_digest(&request.bytes).as_str(),request.profile,request.authority,endpoint])
            .map_err(|_|storage("macro request plan insert"))?;
        let request_plan_version = lease.head;
        if let Some(episode) = episode {
            episode.validate()?;
            let control = episode.controls();
            require(
                control.iter().all(|request| request.endpoint() == endpoint)
                    && control
                        .iter()
                        .all(|request| Some(request.authority()) == plan.acquisition_authority()),
            )?;
            let episode_bytes = codec::encode(&episode)?;
            let episode_digest = raw_digest(&episode_bytes);
            let prior = advance(&transaction, &mut lease, now)?;
            transaction.execute("INSERT INTO chain_post_close_macro_readiness_episode_plans(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,episode_ordinal,plan_version,request_plan_version,phase,item_ordinal,candidate_ordinal,required_operation,endpoint,acquisition_authority,health_request_sha256,capabilities_request_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?14,'Gateway',1,1,?15,?16,?17,?18,?19)",
                params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),episode_bytes,episode_bytes.len(),episode_digest.as_str(),plan_version,request_plan_version,ExternalOperation::GlobalNews as i32,endpoint,plan.acquisition_authority(),raw_digest(control[0].request_bytes()).as_str(),raw_digest(control[1].request_bytes()).as_str()])
                .map_err(|_|storage("macro readiness plan insert"))?;
        }
        inspect_run_on(&transaction, &lease.intent_id)?;
        transaction
            .commit()
            .map_err(|_| storage("macro plan commit"))?;
        Ok(lease)
    }

    pub(super) fn begin_macro_attempt(
        &mut self,
        lease: RunLease,
        attempt: &AuthorizedMacroAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, Call)> {
        self.begin_macro_request(
            lease,
            codec::Request::capture(attempt)?,
            attempt.attempt_ordinal(),
            now,
        )
    }

    pub(super) fn begin_health_control(
        &mut self,
        lease: RunLease,
        attempt: &AuthorizedHealthAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, ControlCall)> {
        self.begin_macro_control(lease, attempt.request_material(), now)
    }

    pub(super) fn begin_capabilities_control(
        &mut self,
        lease: RunLease,
        attempt: &AuthorizedCapabilitiesAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, ControlCall)> {
        self.begin_macro_control(lease, attempt.request_material(), now)
    }

    fn begin_macro_control(
        &mut self,
        mut lease: RunLease,
        material: ExternalControlRequestMaterial,
        now: UtcMicros,
    ) -> Result<(RunLease, ControlCall)> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("macro control begin"))?;
        let (run, recovery) = admission(&transaction, &lease, now)?;
        let recovery = recovery.ok_or(ChainPostCloseError::MacroNotStarted)?;
        require(
            !recovery.has_unconfirmed_effect()
                && recovery.source.is_none()
                && recovery.attempts.is_empty()
                && now.get() < recovery.plan.deadline
                && recovery.plan.profile()
                    == crate::grpc_client::client::ContractProfile::ExternalV1,
        )?;
        let episode = recovery
            .readiness_episodes
            .first()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let control_ordinal = match material.kind {
            ExternalControlKind::Health => 1,
            ExternalControlKind::Capabilities => 2,
        };
        let index = usize::try_from(control_ordinal - 1)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let control = episode
            .controls
            .get(index)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(
            control.begin.is_none()
                && codec::encode(&codec::ControlRequest::capture(material.clone())?)?
                    == codec::encode(&control.request)?,
        )?;
        let health_result = if control_ordinal == 2 {
            let health = &episode.controls[0];
            require(health.outcome == Some(MacroControlOutcome::Ready))?;
            health.result
        } else {
            None
        };
        let request_sha = raw_digest(control.request.request_bytes())
            .as_str()
            .to_owned();
        let bytes = codec::encode(&ControlBegin {
            version: 1,
            episode_ordinal: 1,
            control_ordinal,
            kind: match material.kind {
                ExternalControlKind::Health => "Health",
                ExternalControlKind::Capabilities => "Capabilities",
            }
            .to_owned(),
            episode_sha256: raw_digest(&codec::encode(&episode.plan)?)
                .as_str()
                .to_owned(),
            request_sha256: request_sha.clone(),
            health_result_version: health_result,
        })?;
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute("INSERT INTO chain_post_close_macro_control_attempt_begins(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,episode_ordinal,control_ordinal,kind,episode_plan_version,request_sha256,health_result_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?14,?15,?16,?17)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),bytes,bytes.len(),raw_digest(&bytes).as_str(),control_ordinal,match material.kind { ExternalControlKind::Health=>"Health", ExternalControlKind::Capabilities=>"Capabilities" },episode.plan_version,request_sha,health_result])
            .map_err(|_|storage("macro control begin insert"))?;
        inspect_run_on(&transaction, &lease.intent_id)?;
        transaction
            .commit()
            .map_err(|_| storage("macro control begin commit"))?;
        let call = ControlCall {
            intent: lease.intent_id.as_str().to_owned(),
            run_id: lease.run_id.as_str().to_owned(),
            context: run.context.canonical_sha256().as_str().to_owned(),
            input: raw_digest(&run.input.encode()?).as_str().to_owned(),
            episode_ordinal: 1,
            control_ordinal,
            begin: lease.head,
            request_sha,
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
        };
        Ok((lease, call))
    }

    pub(super) fn record_health_control_result(
        &mut self,
        lease: RunLease,
        call: ControlCall,
        completion: &ExternalControlCompletion<HealthResponse>,
        now: UtcMicros,
    ) -> Result<(RunLease, MacroControlOutcome)> {
        require(call.control_ordinal == 1)?;
        self.record_macro_control_result(
            lease,
            call,
            codec::ControlRawResult::capture_health(completion),
            now,
        )
    }

    pub(super) fn record_capabilities_control_result(
        &mut self,
        lease: RunLease,
        call: ControlCall,
        completion: &ExternalControlCompletion<CapabilitiesResponse>,
        now: UtcMicros,
    ) -> Result<(RunLease, MacroControlOutcome)> {
        require(call.control_ordinal == 2)?;
        self.record_macro_control_result(
            lease,
            call,
            codec::ControlRawResult::capture_capabilities(completion),
            now,
        )
    }

    fn record_macro_control_result(
        &mut self,
        mut lease: RunLease,
        call: ControlCall,
        raw: codec::ControlRawResult,
        now: UtcMicros,
    ) -> Result<(RunLease, MacroControlOutcome)> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("macro control result"))?;
        let (run, recovery) = admission(&transaction, &lease, now)?;
        let recovery = recovery.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let episode = recovery
            .readiness_episodes
            .first()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let index = usize::try_from(call.control_ordinal - 1)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let control = episode
            .controls
            .get(index)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(
            recovery.has_unconfirmed_effect()
                && control.begin == Some(call.begin)
                && control.result.is_none()
                && call.intent == lease.intent_id.as_str()
                && call.run_id == lease.run_id.as_str()
                && call.context == run.context.canonical_sha256().as_str()
                && call.input == raw_digest(&run.input.encode()?).as_str()
                && call.owner == lease.owner.as_str()
                && call.generation == lease.generation
                && call.episode_ordinal == 1
                && call.request_sha == raw_digest(control.request.request_bytes()).as_str(),
        )?;
        let projected = raw.project(&control.request)?;
        let outcome = if projected.is_ok() {
            MacroControlOutcome::Ready
        } else {
            MacroControlOutcome::Rejected
        };
        let bytes = codec::encode(&raw)?;
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute("INSERT INTO chain_post_close_macro_control_attempt_results(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,episode_ordinal,control_ordinal,kind,begin_version,request_sha256,outcome) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?14,?15,?16,?17)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),bytes,bytes.len(),raw_digest(&bytes).as_str(),call.control_ordinal,match control.request.kind(){ExternalControlKind::Health=>"Health",ExternalControlKind::Capabilities=>"Capabilities"},call.begin,call.request_sha,match outcome {MacroControlOutcome::Ready=>"Ready",MacroControlOutcome::Rejected=>"Rejected"}])
            .map_err(|_|storage("macro control result insert"))?;
        if let Err(error) = projected {
            let gateway = Err(error);
            let result_version = lease.head;
            Self::insert_macro_source_final(
                &transaction,
                &mut lease,
                &run,
                "ExternalControlResult",
                result_version,
                &gateway,
                now,
            )?;
        }
        inspect_run_on(&transaction, &lease.intent_id)?;
        transaction
            .commit()
            .map_err(|_| storage("macro control result commit"))?;
        Ok((lease, outcome))
    }

    pub(super) fn begin_prepared_macro_attempt(
        &mut self,
        lease: RunLease,
        attempt: &AuthorizedPreparedMacroRequest,
        now: UtcMicros,
    ) -> Result<(RunLease, Call)> {
        self.begin_macro_request(
            lease,
            codec::Request::capture_prepared(attempt)?,
            attempt.attempt_ordinal(),
            now,
        )
    }

    fn begin_macro_request(
        &mut self,
        mut lease: RunLease,
        request: codec::Request,
        attempt_ordinal: u32,
        now: UtcMicros,
    ) -> Result<(RunLease, Call)> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("macro begin"))?;
        let (run, recovery) = admission(&transaction, &lease, now)?;
        let recovery = recovery.ok_or(ChainPostCloseError::MacroNotStarted)?;
        require(
            !recovery.has_unconfirmed_effect()
                && recovery.source.is_none()
                && now.get() < recovery.plan.deadline,
        )?;
        let expected = u32::try_from(recovery.attempts.len() + 1)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        require(
            expected == attempt_ordinal
                && codec::encode(&request)? == codec::encode(&recovery.plan.request)?,
        )?;
        let last = recovery.attempts.last();
        require(last.is_none_or(|a| {
            matches!(a.continuation(), Some(MacroContinuation::Retry { .. }))
                && a.retry_not_before.is_some_and(|due| due <= now.get())
        }))?;
        let request_sha = raw_digest(&request.bytes).as_str().to_owned();
        let readiness_result =
            if recovery.plan.profile() == crate::grpc_client::client::ContractProfile::ExternalV1 {
                recovery
                    .readiness_episodes
                    .first()
                    .and_then(MacroReadinessEpisodeRecovery::ready_result_version)
                    .ok_or(ChainPostCloseError::SchemaRejected)
                    .map(Some)?
            } else {
                None
            };
        let previous = last.and_then(|a| a.result);
        let bytes = codec::encode(&Begin {
            version: 1,
            ordinal: expected,
            plan_sha256: raw_digest(&recovery.plan_bytes).as_str().to_owned(),
            request_sha256: request_sha.clone(),
            previous_result: previous,
        })?;
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute("INSERT INTO chain_post_close_macro_attempt_begins(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,phase,item_ordinal,candidate_ordinal,attempt_ordinal,request_plan_version,request_sha256,readiness_result_version,previous_result_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'Gateway',1,1,?13,?14,?15,?16,?17)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),bytes,bytes.len(),raw_digest(&bytes).as_str(),expected,recovery.request_plan_version,request_sha,readiness_result,previous])
            .map_err(|_|storage("macro begin insert"))?;
        inspect_run_on(&transaction, &lease.intent_id)?;
        transaction
            .commit()
            .map_err(|_| storage("macro begin commit"))?;
        let call = Call {
            intent: lease.intent_id.as_str().to_owned(),
            run_id: lease.run_id.as_str().to_owned(),
            context: run.context.canonical_sha256().as_str().to_owned(),
            input: raw_digest(&run.input.encode()?).as_str().to_owned(),
            ordinal: expected,
            begin: lease.head,
            request_sha,
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
            readiness_result,
        };
        Ok((lease, call))
    }

    pub(super) fn record_macro_result(
        &mut self,
        lease: RunLease,
        call: Call,
        completion: &MacroAttemptCompletion,
        now: UtcMicros,
    ) -> Result<RunLease> {
        let raw = RawResult::capture(completion);
        let expected = codec::gateway_for(
            crate::grpc_client::client::ContractProfile::LocalBridgeV1,
            &completion.processed,
        );
        self.record_macro_raw_result(lease, call, raw, expected, now)
    }

    pub(super) fn record_external_macro_result(
        &mut self,
        lease: RunLease,
        call: Call,
        completion: &ExternalMacroAttemptCompletion,
        now: UtcMicros,
    ) -> Result<RunLease> {
        let raw = RawResult::capture_external(completion);
        let expected = match completion {
            ExternalMacroAttemptCompletion::Unary(completion) => codec::gateway_for(
                crate::grpc_client::client::ContractProfile::ExternalV1,
                &completion.processed,
            ),
            ExternalMacroAttemptCompletion::ConnectUnavailable { error, .. } => {
                Err(crate::data_gateway::grpc_source::map_external_query_error(
                    crate::grpc_client::pb::magic::market::v1::Operation::GlobalNews,
                    error,
                ))
            }
        };
        self.record_macro_raw_result(lease, call, raw, expected, now)
    }

    fn record_macro_raw_result(
        &mut self,
        mut lease: RunLease,
        call: Call,
        raw: RawResult,
        expected_gateway: NewsResult,
        now: UtcMicros,
    ) -> Result<RunLease> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("macro result begin"))?;
        let (run, recovery) = admission(&transaction, &lease, now)?;
        let recovery = recovery.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let last = recovery
            .attempts
            .last()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(
            recovery.has_unconfirmed_effect()
                && last.ordinal == call.ordinal
                && last.begin == call.begin
                && call.intent == lease.intent_id.as_str()
                && call.run_id == lease.run_id.as_str()
                && call.context == run.context.canonical_sha256().as_str()
                && call.input == raw_digest(&run.input.encode()?).as_str()
                && call.owner == lease.owner.as_str()
                && call.generation == lease.generation
                && call.readiness_result == last.readiness_result
                && call.request_sha == raw_digest(&recovery.plan.request.bytes).as_str(),
        )?;
        let provider_catalog =
            historical_provider_catalog(&recovery.readiness_episodes, last.readiness_result);
        let (gateway, _, _) =
            raw.project(&recovery.plan.request, call.ordinal, provider_catalog)?;
        require(codec::native_bytes(&gateway)? == codec::native_bytes(&expected_gateway)?)?;
        let bytes = codec::encode(&raw)?;
        let (continuation, due) = match raw.continuation() {
            MacroContinuation::Retry { backoff_ms } => {
                let delay = i64::try_from(backoff_ms)
                    .ok()
                    .and_then(|n| n.checked_mul(1000))
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                (
                    "Retry",
                    Some(
                        now.get()
                            .checked_add(delay)
                            .ok_or(ChainPostCloseError::SchemaRejected)?,
                    ),
                )
            }
            MacroContinuation::Terminal => ("Terminal", None),
        };
        let prior = advance(&transaction, &mut lease, now)?;
        transaction.execute("INSERT INTO chain_post_close_macro_attempt_results(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,phase,item_ordinal,candidate_ordinal,attempt_ordinal,begin_version,request_sha256,continuation,retry_not_before) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'Gateway',1,1,?13,?14,?15,?16,?17)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),bytes,bytes.len(),raw_digest(&bytes).as_str(),call.ordinal,call.begin,call.request_sha,continuation,due])
            .map_err(|_|storage("macro result insert"))?;
        if matches!(raw.continuation(), MacroContinuation::Terminal) {
            let result_version = lease.head;
            Self::insert_macro_source_final(
                &transaction,
                &mut lease,
                &run,
                "DataResult",
                result_version,
                &gateway,
                now,
            )?;
        }
        inspect_run_on(&transaction, &lease.intent_id)?;
        transaction
            .commit()
            .map_err(|_| storage("macro result commit"))?;
        Ok(lease)
    }

    fn insert_macro_source_final(
        transaction: &Transaction<'_>,
        lease: &mut RunLease,
        run: &RunRecovery,
        cause_kind: &str,
        cause_version: u64,
        gateway: &NewsResult,
        now: UtcMicros,
    ) -> Result<()> {
        let native = codec::native_bytes(gateway)?;
        let audit = map_gateway_audit_record(
            CAPABILITY,
            GlobalNewsProvider::Eastmoney.provider_id(),
            REQUEST_HASH,
            gateway,
            &audit_time(now.get())?,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let receipt = append_acquisition_in_transaction(transaction, &audit.borrowed(CAPABILITY))
            .map_err(|_| storage("macro audit append"))?;
        verify_acquisition_receipt_in_transaction(
            transaction,
            &receipt,
            &audit.borrowed(CAPABILITY),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let final_ = SourceFinal {
            version: 1,
            phase: "Gateway".to_owned(),
            item_ordinal: 1,
            cause_kind: cause_kind.to_owned(),
            cause_version,
            native_sha256: raw_digest(&native).as_str().to_owned(),
        };
        let bytes = codec::encode(&final_)?;
        let prior = advance(transaction, lease, now)?;
        require(prior == cause_version)?;
        transaction.execute("INSERT INTO chain_post_close_macro_source_finals(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,phase,item_ordinal,cause_kind,cause_version,native_bytes,native_length,native_sha256,audit_id,audit_record_hash,previous_outcome,current_outcome) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'Gateway',1,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
            params![lease.intent_id.as_str(),lease.run_id.as_str(),run.context.canonical_sha256().as_str(),raw_digest(&run.input.encode()?).as_str(),lease.owner.as_str(),lease.generation,prior,lease.head,now.get(),bytes,bytes.len(),raw_digest(&bytes).as_str(),cause_kind,cause_version,native,native.len(),raw_digest(&native).as_str(),receipt.audit_id,receipt.record_hash,receipt.previous_outcome,receipt.current_outcome])
            .map_err(|_|storage("macro source final insert"))?;
        Ok(())
    }
}
