//! Full-layout Macro read model. All facts are checked in the caller's snapshot.
use super::{
    dragon_tiger, macro_codec as codec, macro_native as native, macro_plan_v3 as plan3,
    macro_stage::{
        self as old, Fact, FactExpectedDigests, MacroAttemptRecovery, MacroNewsRecovery,
        MacroRecovery,
    },
    storage, ChainPostCloseError, RunRecovery,
};
use crate::data_gateway::GlobalNewsProvider;
use crate::database::data_acquisition_audit::{
    verify_acquisition_receipt_in_transaction, DataAcquisitionAuditReceipt,
};
use crate::grpc_client::client::{macro_attempt::MacroContinuation, ContractProfile};
use crate::monitor::push_job::{raw_digest, IntentId};
use crate::search_service::macro_news::{
    runner::{
        self, BudgetMode, Dimension, QueryKey, QueryOutcome, QueryState, RouteState, Snapshot,
    },
    NativeOutcome,
};
use codec::{require, Result};
use rusqlite::{params, Transaction};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const TABLES: [&str; 4] = [
    "chain_post_close_macro_query_terminals",
    "chain_post_close_macro_dimension_terminals",
    "chain_post_close_macro_finalize_begins",
    "chain_post_close_macro_stage_finals",
];

pub(super) struct RequestRecovery {
    pub(super) fact: Fact,
    pub(super) request: codec::Request,
    pub(super) endpoint: String,
}

pub(crate) struct QueryTerminalRecovery {
    pub(super) key: QueryKey,
    pub(super) version: u64,
    pub(super) time: i64,
    owner: String,
    generation: u64,
    request_version: Option<u64>,
    pub(super) native: NativeOutcome,
    pub(super) native_bytes: Vec<u8>,
    pub(super) receipt: Option<DataAcquisitionAuditReceipt>,
    pub(super) cause: native::TerminalCause,
}
impl QueryTerminalRecovery {
    pub(crate) fn owner(&self) -> &str { &self.owner }
    pub(crate) fn generation(&self) -> u64 { self.generation }
    pub(crate) fn request_version(&self) -> Option<u64> { self.request_version }
    pub(crate) fn historical_rejection(&self) -> Option<HistoricalRejectionLink<'_>> {
        match &self.cause {
            native::TerminalCause::HistoricalControlRejected {
                control_result_version, control_result_sha256, source_final_version, source_final_sha256,
            } => Some(HistoricalRejectionLink {
                control_result_version: *control_result_version, control_result_sha256,
                source_final_version: *source_final_version, source_final_sha256,
            }),
            _ => None,
        }
    }
    pub(crate) fn query_key(&self) -> QueryKey {
        self.key
    }
    pub(crate) fn version(&self) -> u64 {
        self.version
    }
    pub(crate) fn recorded_at(&self) -> i64 {
        self.time
    }
    pub(crate) fn native(&self) -> &NativeOutcome {
        &self.native
    }
    pub(crate) fn native_bytes(&self) -> &[u8] {
        &self.native_bytes
    }
    pub(crate) fn audit_receipt(&self) -> Option<&DataAcquisitionAuditReceipt> {
        self.receipt.as_ref()
    }
    pub(crate) fn was_called(&self) -> bool {
        matches!(self.cause, native::TerminalCause::DataResult { .. })
    }
}

pub(crate) struct HistoricalRejectionLink<'a> {
    pub(crate) control_result_version: u64,
    pub(crate) control_result_sha256: &'a str,
    pub(crate) source_final_version: u64,
    pub(crate) source_final_sha256: &'a str,
}

/// Original validated journal facts, never reconstructed from a new terminal.
pub(crate) struct HistoricalFact {
    fact: Fact,
}
impl HistoricalFact {
    pub(crate) fn bytes(&self) -> &[u8] { &self.fact.bytes }
    pub(crate) fn sha256(&self) -> &str { &self.fact.digest }
    pub(crate) fn version(&self) -> u64 { self.fact.version }
    pub(crate) fn recorded_at(&self) -> i64 { self.fact.time }
    pub(crate) fn owner(&self) -> &str { &self.fact.owner }
    pub(crate) fn generation(&self) -> u64 { self.fact.generation }
}
pub(crate) struct HistoricalRejectionOrigin {
    control: HistoricalFact,
    source: HistoricalFact,
    pub(super) error: crate::data_gateway::GatewayError,
}
impl HistoricalRejectionOrigin {
    pub(crate) fn control(&self) -> &HistoricalFact { &self.control }
    pub(crate) fn source(&self) -> &HistoricalFact { &self.source }
    pub(super) fn cause(&self) -> native::TerminalCause {
        native::TerminalCause::HistoricalControlRejected {
            control_result_version: self.control.version(),
            control_result_sha256: self.control.sha256().to_owned(),
            source_final_version: self.source.version(),
            source_final_sha256: self.source.sha256().to_owned(),
        }
    }
}

/// Borrowed evidence from the original B, including its canonical bytes.
pub(crate) struct FinalizeBeginRecovery<'a> {
    fact: &'a Fact,
    value: &'a native::FinalizeBegin,
}
impl FinalizeBeginRecovery<'_> {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.fact.bytes
    }
    pub(crate) fn version(&self) -> u64 {
        self.fact.version
    }
    pub(crate) fn kind(&self) -> native::FinalKind {
        self.value.kind
    }
    pub(crate) fn recorded_at(&self) -> i64 {
        self.fact.time
    }
    pub(crate) fn owner(&self) -> &str {
        &self.fact.owner
    }
    pub(crate) fn generation(&self) -> u64 {
        self.fact.generation
    }
    pub(crate) fn expiry(&self) -> &native::ExpiryBasis {
        &self.value.expiry
    }
    pub(crate) fn pending(&self) -> &[QueryKey] {
        &self.value.pending
    }
}

pub(crate) struct StageFinalRecovery {
    begin_bytes: Vec<u8>,
    pub(super) fact: Fact,
    pub(super) begin: native::FinalizeBegin,
    pub(super) value: native::StageFinal,
}
impl StageFinalRecovery {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.fact.bytes
    }
    pub(crate) fn finalize_begin_bytes(&self) -> &[u8] {
        &self.begin_bytes
    }
    pub(crate) fn output_bytes(&self) -> &[u8] {
        &self.begin.output
    }
    pub(crate) fn kind(&self) -> native::FinalKind {
        self.value.kind
    }
    pub(crate) fn version(&self) -> u64 {
        self.fact.version
    }
    pub(crate) fn finalize_begin_version(&self) -> u64 {
        self.value.finalize_begin_version
    }
    pub(crate) fn plan_version(&self) -> u64 {
        self.begin.plan_version
    }
    pub(crate) fn started_at(&self) -> i64 {
        self.begin.started_at
    }
    pub(crate) fn deadline_at(&self) -> i64 {
        self.begin.deadline_at
    }
    pub(crate) fn recorded_at(&self) -> i64 {
        self.fact.time
    }
    pub(crate) fn owner(&self) -> &str {
        &self.fact.owner
    }
    pub(crate) fn generation(&self) -> u64 {
        self.fact.generation
    }
}

pub(super) struct FullRecovery {
    pub(super) historical_rejection: Option<HistoricalRejectionOrigin>,
    pub(super) pending_sources: Vec<crate::grpc_client::client::macro_attempt::MacroQueryIdentity>,
    pub(super) pending_research: Vec<String>,
    pub(super) local: plan3::LocalRoute,
    pub(super) requests: BTreeMap<QueryKey, RequestRecovery>,
    pub(super) terminals: BTreeMap<QueryKey, QueryTerminalRecovery>,
    pub(super) dimensions: BTreeMap<u8, (Fact, native::DimensionTerminal)>,
    pub(super) begin: Option<(Fact, native::FinalizeBegin)>,
    pub(super) final_: Option<StageFinalRecovery>,
    pub(super) news: Vec<(GlobalNewsProvider, MacroNewsRecovery)>,
    pub(super) snapshot: Snapshot,
    pub(super) facts_sha256: String,
}

impl MacroRecovery {
    pub(crate) fn historical_rejection_origin(&self) -> Option<&HistoricalRejectionOrigin> {
        self.full.as_ref()?.historical_rejection.as_ref()
    }
    pub(crate) fn finalize_begin(&self) -> Option<FinalizeBeginRecovery<'_>> {
        let (fact, value) = self.full.as_ref()?.begin.as_ref()?;
        Some(FinalizeBeginRecovery { fact, value })
    }
    pub(crate) fn query_terminal(&self, key: QueryKey) -> Option<&QueryTerminalRecovery> {
        self.full.as_ref()?.terminals.get(&key)
    }
    pub(crate) fn stage_final(&self) -> Option<&StageFinalRecovery> {
        self.full.as_ref()?.final_.as_ref()
    }
}

fn key(phase: &str, item: u8, candidate: u32) -> Result<QueryKey> {
    let key = match phase {
        "Gateway" if candidate == 1 => QueryKey::Gateway(item),
        "WebDimension" => QueryKey::Web {
            dimension: item,
            candidate,
        },
        _ => return Err(ChainPostCloseError::SchemaRejected),
    };
    native::query_columns(key)?;
    Ok(key)
}

fn bound(fact: &Fact, plan: &codec::Plan) -> Result<()> {
    require(fact.time >= plan.started && fact.time < plan.deadline)
}

pub(super) fn load_on<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    run: &'run RunRecovery,
    validated: Option<&dragon_tiger::ValidatedDragonTiger<'transaction, 'connection, 'run>>,
) -> Result<Option<MacroRecovery>> {
    let mut groups = old::TABLES
        .iter()
        .chain(TABLES.iter())
        .map(|table| old::facts(transaction, intent, table))
        .collect::<Result<Vec<_>>>()?;
    if groups[0].is_empty() {
        require(groups.iter().all(Vec::is_empty))?;
        return Ok(None);
    }
    require(groups[0].len() == 1 && groups[10].len() <= 1 && groups[11].len() <= 1)?;
    let mut expected = FactExpectedDigests::new(run);
    for fact in groups.iter().flatten() {
        fact.validate_with_expected(&mut expected)?;
    }
    let fact = groups[0].remove(0);
    let parent = match validated {
        Some(validated) => {
            dragon_tiger::macro_parent_from_validated(transaction, intent, run, validated)?
        }
        None => dragon_tiger::macro_parent(transaction, intent, run)?,
    };
    let value: serde_json::Value =
        serde_json::from_slice(&fact.bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let (mut recovery, local) =
        if value.get("version").and_then(serde_json::Value::as_u64) == Some(3) {
            let plan = plan3::PlanV3::decode(&fact.bytes, &parent)?;
            require(groups[7].is_empty())?;
            (
                MacroRecovery {
                    full: None,
                    plan: plan.core,
                    plan_bytes: fact.bytes.clone(),
                    plan_version: fact.version,
                    request_plan_version: 0,
                    attempts: Vec::new(),
                    readiness_episodes: Vec::new(),
                    source: None,
                },
                plan.local_route,
            )
        } else {
            let legacy = old::load_legacy_subset(transaction, intent, run, validated)?
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            let local = match plan3::legacy_local_route(&legacy.plan) {
                Ok(local) => local,
                Err(
                    ChainPostCloseError::LegacyPlanExecutionUnsupported
                    | ChainPostCloseError::MissingPersistedLocalRoute,
                ) => {
                    require(
                        groups[8..].iter().all(Vec::is_empty)
                            && groups[1].len() == 1
                            && groups[5].len() == legacy.attempts.len(),
                    )?;
                    return Ok(Some(legacy));
                }
                Err(error) => return Err(error),
            };
            (legacy, local)
        };
    let plan = &recovery.plan;
    let definition = plan3::definition(plan)?;
    let plan_extra: (u64, String, i64, i64, String) = transaction.query_row(
        "SELECT parent_version,parent_sha256,started_at,deadline_at,request_sha256 FROM chain_post_close_macro_plans WHERE intent_id=?1",
        [intent.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))
        .map_err(|_| storage("full macro plan"))?;
    require(
        plan_extra
            == (
                plan.parent_version,
                plan.parent_digest.clone(),
                plan.started,
                plan.deadline,
                raw_digest(&plan.request.bytes).as_str().to_owned(),
            )
            && fact.prior >= parent.version,
    )?;
    bound(&fact, plan)?;
    let mut requests = BTreeMap::new();
    for request_fact in std::mem::take(&mut groups[1]) {
        bound(&request_fact, plan)?;
        let extra: (String, u8, u32, u64, String, String, Option<String>, String) = transaction.query_row(
            "SELECT phase,item_ordinal,candidate_ordinal,plan_version,request_sha256,profile,acquisition_authority,endpoint FROM chain_post_close_macro_request_plans WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(), request_fact.version], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)))
            .map_err(|_| storage("full macro request"))?;
        let query = key(&extra.0, extra.1, extra.2)?;
        let value: serde_json::Value = serde_json::from_slice(&request_fact.bytes)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let request = if value.get("version").is_some() {
            let request: plan3::RequestPlan = codec::decode(&request_fact.bytes)?;
            require(request.query == query)?;
            request.validate(&definition, &local, plan.endpoint(), &extra.7)?;
            request.request
        } else {
            require(
                query == QueryKey::Gateway(1)
                    && recovery.request_plan_version == request_fact.version,
            )?;
            codec::decode(&request_fact.bytes)?
        };
        require(
            extra.3 == fact.version
                && extra.4 == raw_digest(&request.bytes).as_str()
                && extra.5 == request.profile
                && extra.6 == request.authority
                && request_fact.prior >= fact.version,
        )?;
        if query == QueryKey::Gateway(1) {
            require(codec::encode(&request)? == codec::encode(&plan.request)?)?;
            recovery.request_plan_version = request_fact.version;
        }
        require(
            requests
                .insert(
                    query,
                    RequestRecovery {
                        fact: request_fact,
                        request,
                        endpoint: extra.7,
                    },
                )
                .is_none(),
        )?;
    }
    let first = requests
        .get(&QueryKey::Gateway(1))
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let (episodes, rejected, ready_time) = old::recover_readiness(
        transaction,
        intent,
        plan,
        &fact,
        &first.fact,
        &groups[2],
        &groups[3],
        &groups[4],
        true,
    )?;
    if definition.external_news
        && value.get("version").and_then(serde_json::Value::as_u64) == Some(3)
    {
        require((1..=4).all(|ordinal| {
            requests
                .get(&QueryKey::Gateway(ordinal))
                .is_some_and(|request| {
                    request.fact.owner == fact.owner
                        && request.fact.generation == fact.generation
                        && request.fact.time == fact.time
                })
        }))?;
    }
    recovery.readiness_episodes = episodes;
    let ready = recovery
        .readiness_episodes
        .first()
        .and_then(|episode| episode.ready_result_version());
    let mut query_states = BTreeMap::<QueryKey, QueryState>::new();
    for attempt in &recovery.attempts {
        let state = query_states.entry(attempt.query).or_default();
        state.next_attempt = attempt.ordinal + 1;
        state.retry_due = attempt.retry_not_before;
    }
    let legacy_begins = recovery
        .attempts
        .iter()
        .map(|attempt| attempt.begin)
        .collect::<BTreeSet<_>>();
    let legacy_results = recovery
        .attempts
        .iter()
        .filter_map(|attempt| attempt.result)
        .collect::<BTreeSet<_>>();
    let mut result_by_begin = BTreeMap::new();
    for result in &groups[6] {
        if legacy_results.contains(&result.version) {
            continue;
        }
        let begin: u64 = transaction.query_row("SELECT begin_version FROM chain_post_close_macro_attempt_results WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(), result.version], |row| row.get(0)).map_err(|_| storage("full macro result link"))?;
        require(result_by_begin.insert(begin, result).is_none())?;
    }
    let mut terminal_results = BTreeMap::new();
    for begin in &groups[5] {
        if legacy_begins.contains(&begin.version) {
            continue;
        }
        bound(begin, plan)?;
        let decoded: native::DataBegin = codec::decode(&begin.bytes)?;
        let request = requests
            .get(&decoded.query)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let identity = definition
            .identity(decoded.query)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let extra: (String,u8,u32,u32,u64,String,Option<u64>,Option<u64>) = transaction.query_row(
            "SELECT phase,item_ordinal,candidate_ordinal,attempt_ordinal,request_plan_version,request_sha256,readiness_result_version,previous_result_version FROM chain_post_close_macro_attempt_begins WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(),begin.version], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)))
            .map_err(|_| storage("full macro begin"))?;
        let previous = recovery
            .attempts
            .iter()
            .rev()
            .find(|attempt| attempt.query == decoded.query);
        require(
            decoded.version == 2
                && key(&extra.0, extra.1, extra.2)? == decoded.query
                && extra.3 == decoded.attempt
                && decoded.attempt == previous.map_or(1, |a| a.ordinal + 1)
                && decoded.attempt <= request.request.policy.0
                && extra.4 == request.fact.version
                && extra.4 == decoded.request_plan_version
                && extra.5 == decoded.request_sha256
                && extra.5 == raw_digest(&request.request.bytes).as_str()
                && extra.6 == decoded.readiness_result_version
                && extra.7 == decoded.previous_result_version
                && extra.7 == previous.and_then(|a| a.result)
                && begin.prior >= request.fact.version
                && begin.time >= request.fact.time
                && previous.is_none_or(|a| {
                    a.result.is_some()
                        && matches!(a.continuation(), Some(MacroContinuation::Retry { .. }))
                        && a.retry_not_before.is_some_and(|due| due <= begin.time)
                }),
        )?;
        if request.request.contract_profile() == ContractProfile::ExternalV1 {
            require(
                extra.6.is_some()
                    && extra.6 == ready
                    && ready.is_some_and(|version| version <= begin.prior)
                    && ready_time.is_some_and(|time| time <= begin.time),
            )?;
        } else {
            require(extra.6.is_none())?;
        }
        let provider_catalog =
            old::historical_provider_catalog(&recovery.readiness_episodes, extra.6);
        let mut attempt = MacroAttemptRecovery {
            query: decoded.query,
            request: request.request.bytes.clone(),
            request_id: request.request.id.clone(),
            ordinal: decoded.attempt,
            begin: begin.version,
            result: None,
            readiness_result: extra.6,
            material: None,
            retry_not_before: None,
        };
        if let Some(result) = result_by_begin.remove(&begin.version) {
            bound(result, plan)?;
            let extra: (String,u8,u32,u32,u64,String,String,Option<i64>) = transaction.query_row(
                "SELECT phase,item_ordinal,candidate_ordinal,attempt_ordinal,begin_version,request_sha256,continuation,retry_not_before FROM chain_post_close_macro_attempt_results WHERE intent_id=?1 AND run_version=?2",
                params![intent.as_str(),result.version], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)))
                .map_err(|_| storage("full macro result"))?;
            let data: native::DataResult = codec::decode(&result.bytes)?;
            require(
                key(&extra.0, extra.1, extra.2)? == decoded.query
                    && data.query == decoded.query
                    && data.attempt == decoded.attempt
                    && extra.3 == decoded.attempt
                    && extra.4 == begin.version
                    && extra.5 == decoded.request_sha256
                    && result.prior >= begin.version
                    && result.time >= begin.time
                    && result.owner == begin.owner
                    && result.generation == begin.generation,
            )?;
            let (outcome, material) =
                data.project(&identity, &request.request, provider_catalog)?;
            match material.continuation {
                MacroContinuation::Terminal => {
                    require(extra.6 == "Terminal" && extra.7.is_none())?;
                    require(
                        terminal_results
                            .insert(decoded.query, (result.clone(), outcome))
                            .is_none(),
                    )?;
                }
                MacroContinuation::Retry { backoff_ms } => {
                    let delay = i64::try_from(backoff_ms)
                        .ok()
                        .and_then(|delay| delay.checked_mul(1000))
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    require(extra.6 == "Retry" && extra.7 == result.time.checked_add(delay))?;
                }
            }
            attempt.result = Some(result.version);
            attempt.material = Some(material);
            attempt.retry_not_before = extra.7;
        }
        let state = query_states.entry(decoded.query).or_default();
        state.next_attempt = decoded.attempt + 1;
        state.retry_due = attempt.retry_not_before;
        recovery.attempts.push(attempt);
    }
    require(result_by_begin.is_empty())?;
    validate_effect_width(&recovery)?;
    finish_recovery(
        transaction,
        intent,
        recovery,
        local,
        requests,
        query_states,
        terminal_results,
        rejected,
        groups,
        fact,
    )
}

/// Validate every historical prefix, not merely the current unmatched suffix.
fn validate_effect_width(recovery: &MacroRecovery) -> Result<()> {
    let mut events = BTreeMap::new();
    let mut add = |begin: u64, result: Option<u64>| -> Result<()> {
        require(events.insert(begin, (begin, true)).is_none())?;
        if let Some(result) = result {
            require(result > begin && events.insert(result, (begin, false)).is_none())?;
        }
        Ok(())
    };
    for attempt in &recovery.attempts {
        add(attempt.begin, attempt.result)?;
    }
    for episode in &recovery.readiness_episodes {
        for control in &episode.controls {
            if let Some(begin) = control.begin {
                add(begin, control.result)?;
            }
        }
    }
    let mut active = BTreeSet::new();
    for (_, (begin, starting)) in events {
        if starting {
            require(active.insert(begin) && active.len() <= 5)?;
        } else {
            require(active.remove(&begin))?;
        }
    }
    Ok(())
}

fn finish_recovery(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    mut recovery: MacroRecovery,
    local: plan3::LocalRoute,
    requests: BTreeMap<QueryKey, RequestRecovery>,
    mut query_states: BTreeMap<QueryKey, QueryState>,
    mut terminal_results: BTreeMap<QueryKey, (Fact, NativeOutcome)>,
    rejected: Option<(u64, crate::data_gateway::GatewayError)>,
    mut groups: Vec<Vec<Fact>>,
    plan_fact: Fact,
) -> Result<Option<MacroRecovery>> {
    let plan = &recovery.plan;
    let definition = plan3::definition(plan)?;
    let is_v3 = serde_json::from_slice::<serde_json::Value>(&plan_fact.bytes)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
        .get("version")
        .and_then(serde_json::Value::as_u64)
        == Some(3);
    let mut terminals = BTreeMap::new();
    let historical_rejection = if !is_v3 && recovery.source.is_some() && rejected.is_some() {
        require(plan.format_version() == 2 && definition.external_news)?;
        require(!recovery.attempts.iter().any(|attempt|
            matches!(attempt.query, QueryKey::Gateway(1..=4))))?;
        let (version, error) = rejected.as_ref().ok_or(ChainPostCloseError::SchemaRejected)?;
        let control = groups[4].iter().find(|fact| fact.version == *version)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let source = groups[7].first().ok_or(ChainPostCloseError::SchemaRejected)?;
        require(source.prior == control.version && source.owner == control.owner
            && source.generation == control.generation && source.time == control.time)?;
        Some(HistoricalRejectionOrigin {
            control: HistoricalFact { fact: control.clone() },
            source: HistoricalFact { fact: source.clone() },
            error: error.clone(),
        })
    } else {
        None
    };
    if let Some(source) = &recovery.source {
        let fact = groups[7]
            .first()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let cause_version: u64 = transaction
            .query_row(
                "SELECT cause_version FROM chain_post_close_macro_source_finals WHERE intent_id=?1",
                [intent.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| storage("historical Macro terminal"))?;
        terminals.insert(
            QueryKey::Gateway(1),
            QueryTerminalRecovery {
                key: QueryKey::Gateway(1),
                version: fact.version,
                time: fact.time,
                owner: fact.owner.clone(),
                generation: fact.generation,
                request_version: Some(recovery.request_plan_version),
                native: NativeOutcome::News(source.result.clone()),
                native_bytes: source.bytes.clone(),
                receipt: Some(source.receipt.clone()),
                cause: if rejected.is_some() {
                    native::TerminalCause::SharedControlRejected {
                        version: cause_version,
                    }
                } else {
                    native::TerminalCause::DataResult {
                        version: cause_version,
                    }
                },
            },
        );
    }
    for fact in &groups[8] {
        bound(fact, plan)?;
        let value: native::QueryTerminal = codec::decode(&fact.bytes)?;
        type Extra = (
            String,
            u8,
            u32,
            u64,
            String,
            Option<u64>,
            Option<String>,
            String,
            Option<u64>,
            Option<u64>,
            String,
            Vec<u8>,
            usize,
            String,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let extra: Extra = transaction.query_row(
            "SELECT phase,item_ordinal,candidate_ordinal,plan_version,plan_sha256,request_plan_version,request_sha256,cause_kind,data_result_version,control_result_version,call_state,native_bytes,native_length,native_sha256,audit_id,audit_record_hash,audit_capability,audit_provider,audit_request_hash,previous_outcome,current_outcome FROM chain_post_close_macro_query_terminals WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(),fact.version], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?,row.get(13)?,row.get(14)?,row.get(15)?,row.get(16)?,row.get(17)?,row.get(18)?,row.get(19)?,row.get(20)?)))
            .map_err(|_| storage("full macro terminal"))?;
        require(
            value.version == 2
                && value.query == key(&extra.0, extra.1, extra.2)?
                && value.plan_version == plan_fact.version
                && extra.3 == plan_fact.version
                && value.plan_sha256 == plan_fact.digest
                && extra.4 == plan_fact.digest
                && value.request_plan_version == extra.5
                && value.request_sha256 == extra.6
                && value.native_sha256 == extra.13
                && extra.11.len() == extra.12
                && raw_digest(&extra.11).as_str() == extra.13,
        )?;
        let request = requests.get(&value.query);
        let outcome = match value.cause {
            native::TerminalCause::DataResult { version } => {
                let (cause, outcome) = terminal_results
                    .remove(&value.query)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                require(
                    extra.7 == "DataResult"
                        && extra.8 == Some(version)
                        && extra.9.is_none()
                        && extra.10 == "Returned"
                        && cause.version == version
                        && fact.prior == version
                        && fact.owner == cause.owner
                        && fact.generation == cause.generation
                        && fact.time == cause.time,
                )?;
                outcome
            }
            native::TerminalCause::SharedControlRejected { version } => {
                let (actual, error) = rejected
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let cause = groups[4]
                    .iter()
                    .find(|fact| fact.version == version)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                require(
                    matches!(value.query, QueryKey::Gateway(1..=4))
                        && *actual == version
                        && extra.7 == "SharedControlRejected"
                        && extra.8.is_none()
                        && extra.9 == Some(version)
                        && extra.10 == "NotCalled"
                        && fact.prior >= version
                        && fact.owner == cause.owner
                        && fact.generation == cause.generation
                        && fact.time == cause.time
                        && !recovery
                            .attempts
                            .iter()
                            .any(|attempt| attempt.query == value.query),
                )?;
                NativeOutcome::News(Err(error.clone()))
            }
            native::TerminalCause::HistoricalControlRejected {
                control_result_version, ref control_result_sha256,
                source_final_version, ref source_final_sha256,
            } => {
                let origin = historical_rejection.as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                require(matches!(value.query, QueryKey::Gateway(2..=4))
                    && control_result_version == origin.control.version()
                    && control_result_sha256 == origin.control.sha256()
                    && source_final_version == origin.source.version()
                    && source_final_sha256 == origin.source.sha256()
                    && extra.7 == "HistoricalControlRejected"
                    && extra.8.is_none() && extra.9 == Some(control_result_version)
                    && extra.10 == "NotCalled" && fact.prior >= source_final_version
                    && fact.time >= origin.source.recorded_at())?;
                NativeOutcome::News(Err(origin.error.clone()))
            }
            native::TerminalCause::RequestRejected => {
                require(
                    matches!(value.query, QueryKey::Web { .. })
                        && extra.7 == "RequestRejected"
                        && extra.8.is_none()
                        && extra.9.is_none()
                        && extra.10 == "NotCalled"
                        && request.is_none(),
                )?;
                native::request_rejected_outcome(&definition, value.query)?
            }
            native::TerminalCause::LocalRouteUnavailable => {
                require(
                    value.query == QueryKey::Gateway(5)
                        && extra.7 == "LocalRouteUnavailable"
                        && extra.8.is_none()
                        && extra.9.is_none()
                        && extra.10 == "NotCalled"
                        && request.is_none()
                        && (!is_v3
                            || (fact.time == plan_fact.time
                                && fact.owner == plan_fact.owner
                                && fact.generation == plan_fact.generation)),
                )?;
                native::local_unavailable_outcome(&local)?
            }
        };
        require(
            extra.5 == request.map(|request| request.fact.version)
                && extra.6
                    == request
                        .map(|request| raw_digest(&request.request.bytes).as_str().to_owned())
                && native::native_bytes(&outcome)? == extra.11,
        )?;
        let receipt = if matches!(value.query, QueryKey::Gateway(_)) {
            let identity = definition
                .identity(value.query)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            let (capability, audit) = native::gateway_audit(&identity, &outcome, fact.time)?;
            require(
                extra.16.as_deref() == Some(capability)
                    && extra.17.as_deref() == Some(audit.provider.as_str())
                    && extra.18.as_deref() == Some(audit.request_hash.as_str()),
            )?;
            let receipt = DataAcquisitionAuditReceipt {
                audit_id: extra.14.ok_or(ChainPostCloseError::SchemaRejected)?,
                record_hash: extra.15.ok_or(ChainPostCloseError::SchemaRejected)?,
                previous_outcome: extra.19,
                current_outcome: extra.20.ok_or(ChainPostCloseError::SchemaRejected)?,
            };
            verify_acquisition_receipt_in_transaction(
                transaction,
                &receipt,
                &audit.borrowed(capability),
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            Some(receipt)
        } else {
            require(
                extra.14.is_none()
                    && extra.15.is_none()
                    && extra.16.is_none()
                    && extra.17.is_none()
                    && extra.18.is_none()
                    && extra.19.is_none()
                    && extra.20.is_none(),
            )?;
            None
        };
        require(
            terminals
                .insert(
                    value.query,
                    QueryTerminalRecovery {
                        key: value.query,
                        version: fact.version,
                        time: fact.time,
                        owner: fact.owner.clone(),
                        generation: fact.generation,
                        request_version: value.request_plan_version,
                        native: outcome,
                        native_bytes: extra.11,
                        receipt,
                        cause: value.cause,
                    },
                )
                .is_none(),
        )?;
    }
    require(terminal_results.is_empty())?;
    if let Some(origin) = &historical_rejection {
        validate_historical_group(origin, plan, &requests, &terminals, &groups[8])?;
    }
    if let Some((version, _)) = &rejected {
        // A v11 prefix retains its original single SourceFinal; once full facts
        // are appended, rejection must be settled for every preregistered lane.
        if recovery.source.is_none() {
            require((1..=4).all(|ordinal| terminals.get(&QueryKey::Gateway(ordinal)).is_some_and(|terminal|
                matches!(terminal.cause, native::TerminalCause::SharedControlRejected { version: actual } if actual == *version))))?;
        }
    }
    if local.state == plan3::LocalRouteState::ObservedUnavailable {
        require(
            (!is_v3 || terminals.contains_key(&QueryKey::Gateway(5)))
                && !requests
                    .keys()
                    .any(|key| matches!(key, QueryKey::Web { .. })),
        )?;
    }
    for (key, terminal) in &terminals {
        let state = query_states.entry(*key).or_default();
        state.terminal = Some(QueryOutcome::Native(terminal.native.clone()));
        state.terminal_version = Some(terminal.version);
        state.terminal_at = Some(terminal.time);
        state.retry_due = None;
    }
    let external = if !definition.external_news {
        RouteState::Ready
    } else if rejected.is_some() {
        RouteState::Rejected
    } else if recovery
        .readiness_episodes
        .first()
        .is_some_and(|episode| episode.ready_result_version().is_some())
    {
        RouteState::Ready
    } else if recovery.readiness_episodes.first().is_some_and(|episode| {
        episode
            .controls()
            .first()
            .is_some_and(|control| control.outcome() == Some(old::MacroControlOutcome::Ready))
    }) {
        RouteState::NeedsCapabilities
    } else {
        RouteState::NeedsHealth
    };
    let mut snapshot = Snapshot {
        budget: BudgetMode::DurableAbsolute {
            started_at: plan.started,
            deadline_at: plan.deadline,
        },
        definition,
        local: if local.state == plan3::LocalRouteState::ObservedConnected {
            RouteState::Ready
        } else {
            RouteState::Rejected
        },
        external,
        queries: query_states,
        dimensions: BTreeMap::new(),
        final_output: None,
    };
    let dimensions = recover_dimensions(
        transaction,
        intent,
        &plan_fact,
        plan,
        &requests,
        &terminals,
        &groups[9],
        &mut snapshot,
    )?;
    let mut digest_material = std::iter::once((plan_fact.version, plan_fact.digest.clone()))
        .chain(
            groups[..10]
                .iter()
                .flatten()
                .map(|fact| (fact.version, fact.digest.clone())),
        )
        .chain(
            requests
                .values()
                .map(|request| (request.fact.version, request.fact.digest.clone())),
        )
        .collect::<Vec<_>>();
    digest_material.sort_by_key(|(version, _)| *version);
    require(digest_material.windows(2).all(|pair| pair[0].0 < pair[1].0))?;
    let facts_sha256 = raw_digest(&codec::encode(&digest_material)?)
        .as_str()
        .to_owned();
    let begin = groups[10]
        .pop()
        .map(|fact| {
            let value: native::FinalizeBegin = codec::decode(&fact.bytes)?;
            value.validate_recorded_at(fact.time)?;
            require(
                value.plan_version == plan_fact.version
                    && value.plan_sha256 == plan_fact.digest
                    && value.started_at == plan.started
                    && value.deadline_at == plan.deadline
                    && value.facts_sha256 == facts_sha256
                    && !recovery
                        .attempts
                        .iter()
                        .any(|attempt| attempt.result.is_none())
                    && !recovery.readiness_episodes.iter().any(|episode| {
                        episode.controls().iter().any(|control| {
                            control.begin_version().is_some() && control.result_version().is_none()
                        })
                    }),
            )?;
            validate_final_columns(transaction, intent, TABLES[2], &fact, &value, None)?;
            match value.kind {
                native::FinalKind::Complete => {
                    bound(&fact, plan)?;
                    require(
                        snapshot.dimensions.len() == 6
                            && snapshot
                                .dimensions
                                .get(&6)
                                .is_some_and(|dimension| dimension.pace_due <= fact.time)
                            && value.output
                                == runner::render(&snapshot)
                                    .map_err(|_| ChainPostCloseError::SchemaRejected)?
                                    .as_bytes(),
                    )?;
                }
                native::FinalKind::BudgetExpired => {
                    require(value.pending == pending(&snapshot))?;
                }
            }
            Ok((fact, value))
        })
        .transpose()?;
    let final_ = groups[11]
        .pop()
        .map(|fact| {
            let (begin_fact, begin) = begin.as_ref().ok_or(ChainPostCloseError::SchemaRejected)?;
            let value: native::StageFinal = codec::decode(&fact.bytes)?;
            value.validate(begin_fact.version, &begin_fact.bytes, begin)?;
            require(
                fact.prior == begin_fact.version
                    && fact.owner == begin_fact.owner
                    && fact.generation == begin_fact.generation
                    && fact.time >= begin_fact.time
                    && (value.kind != native::FinalKind::Complete || fact.time < plan.deadline),
            )?;
            validate_final_columns(
                transaction,
                intent,
                TABLES[3],
                &fact,
                begin,
                Some(begin_fact.version),
            )?;
            snapshot.final_output = Some(
                String::from_utf8(begin.output.clone())
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
            );
            Ok(StageFinalRecovery {
                begin_bytes: begin_fact.bytes.clone(),
                fact,
                begin: begin.clone(),
                value,
            })
        })
        .transpose()?;
    let mut news = Vec::new();
    for (key, terminal) in &terminals {
        if let (QueryKey::Gateway(ordinal @ 1..=4), NativeOutcome::News(result)) =
            (key, &terminal.native)
        {
            let request = &requests
                .get(key)
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .request;
            let provider = [
                GlobalNewsProvider::Eastmoney,
                GlobalNewsProvider::Cailianpress,
                GlobalNewsProvider::Jin10,
                GlobalNewsProvider::ThePaper,
            ][usize::from(*ordinal - 1)];
            news.push((
                provider,
                MacroNewsRecovery {
                    result: result.clone(),
                    bytes: terminal.native_bytes.clone(),
                    receipt: terminal
                        .receipt
                        .clone()
                        .ok_or(ChainPostCloseError::SchemaRejected)?,
                    policy: request.policy,
                    profile: request.profile.clone(),
                    authority: request.authority.clone(),
                },
            ));
        }
    }
    recovery.full = Some(FullRecovery {
        historical_rejection,
        pending_sources: (1..=5)
            .filter(|ordinal| snapshot.terminal(QueryKey::Gateway(*ordinal)).is_none())
            .map(|ordinal| {
                snapshot
                    .definition
                    .identity(QueryKey::Gateway(ordinal))
                    .map_err(|_| ChainPostCloseError::SchemaRejected)
            })
            .collect::<Result<Vec<_>>>()?,
        pending_research: (1..=6)
            .filter(|dimension| !snapshot.dimensions.contains_key(dimension))
            .map(|dimension| snapshot.definition.query(dimension))
            .collect(),
        local,
        requests,
        terminals,
        dimensions,
        begin,
        final_,
        news,
        snapshot,
        facts_sha256,
    });
    Ok(Some(recovery))
}

fn validate_historical_group(
    origin: &HistoricalRejectionOrigin,
    plan: &codec::Plan,
    requests: &BTreeMap<QueryKey, RequestRecovery>,
    terminals: &BTreeMap<QueryKey, QueryTerminalRecovery>,
    terminal_facts: &[Fact],
) -> Result<()> {
    let any = (2..=4).any(|ordinal| requests.contains_key(&QueryKey::Gateway(ordinal))
        || terminals.contains_key(&QueryKey::Gateway(ordinal)));
    if !any {
        return Ok(()); // The original prefix may separately have E or a budget final.
    }
    let mut facts = Vec::with_capacity(6);
    for ordinal in 2..=4 {
        let request = requests.get(&QueryKey::Gateway(ordinal))
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(request.endpoint == plan.endpoint() && request.request.profile == plan.request.profile
            && request.request.authority == plan.request.authority && request.request.policy == plan.request.policy)?;
        facts.push(&request.fact);
    }
    let mut audit_ids = BTreeSet::new();
    for ordinal in 2..=4 {
        let key = QueryKey::Gateway(ordinal);
        let terminal = terminals.get(&key).ok_or(ChainPostCloseError::SchemaRejected)?;
        let link = terminal.historical_rejection().ok_or(ChainPostCloseError::SchemaRejected)?;
        require(link.control_result_version == origin.control.version()
            && link.control_result_sha256 == origin.control.sha256()
            && link.source_final_version == origin.source.version()
            && link.source_final_sha256 == origin.source.sha256()
            && terminal.request_version == Some(facts[usize::from(ordinal - 2)].version))?;
        require(audit_ids.insert(terminal.receipt.as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?.audit_id))?;
        facts.push(terminal_facts.iter().find(|fact| fact.version == terminal.version)
            .ok_or(ChainPostCloseError::SchemaRejected)?);
    }
    let first = facts[0];
    require(first.prior >= origin.source.version() && first.time >= origin.source.recorded_at())?;
    for (index, fact) in facts.iter().enumerate() {
        let prior = first.prior.checked_add(u64::try_from(index)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(fact.prior == prior && fact.version == prior.checked_add(1)
            .ok_or(ChainPostCloseError::SchemaRejected)?
            && fact.owner == first.owner && fact.generation == first.generation
            && fact.time == first.time)?;
    }
    Ok(())
}

pub(super) fn pending(snapshot: &Snapshot) -> Vec<QueryKey> {
    let mut pending = (1..=5)
        .map(QueryKey::Gateway)
        .filter(|key| snapshot.terminal(*key).is_none())
        .collect::<Vec<_>>();
    for dimension in 1..=6 {
        if snapshot.dimensions.contains_key(&dimension) {
            continue;
        }
        pending.extend(
            snapshot
                .definition
                .candidates
                .iter()
                .filter(|candidate| candidate.eligible)
                .map(|candidate| QueryKey::Web {
                    dimension,
                    candidate: candidate.ordinal,
                })
                .filter(|key| snapshot.terminal(*key).is_none()),
        );
    }
    pending
}

fn validate_final_columns(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    table: &str,
    fact: &Fact,
    value: &native::FinalizeBegin,
    linked_begin: Option<u64>,
) -> Result<()> {
    require(TABLES[2..].contains(&table))?;
    let extra: (u64,String,String,i64,i64,String,Vec<u8>,usize,String) = transaction.query_row(
        &format!("SELECT plan_version,plan_sha256,kind,started_at,deadline_at,facts_sha256,output_bytes,output_length,output_sha256 FROM {table} WHERE intent_id=?1"),
        [intent.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?)))
        .map_err(|_| storage("full Macro final"))?;
    require(
        extra.0 == value.plan_version
            && extra.1 == value.plan_sha256
            && extra.2
                == match value.kind {
                    native::FinalKind::Complete => "Complete",
                    native::FinalKind::BudgetExpired => "BudgetExpired",
                }
            && extra.3 == value.started_at
            && extra.4 == value.deadline_at
            && extra.5 == value.facts_sha256
            && extra.6 == value.output
            && extra.7 == value.output.len()
            && extra.8 == value.output_sha256,
    )?;
    if let Some(begin) = linked_begin {
        let actual: u64 = transaction.query_row("SELECT finalize_begin_version FROM chain_post_close_macro_stage_finals WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(),fact.version], |row| row.get(0)).map_err(|_| storage("Macro final begin"))?;
        require(actual == begin)?;
    } else {
        require(table == TABLES[2])?;
        let actual: (Option<i64>, Option<i64>) = transaction.query_row(
            "SELECT expiry_opened_at,expiry_elapsed_us FROM chain_post_close_macro_finalize_begins WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(), fact.version], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| storage("Macro finalize expiry"))?;
        require(actual == value.expiry_columns())?;
    }
    Ok(())
}

fn recover_dimensions(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    plan_fact: &Fact,
    plan: &codec::Plan,
    requests: &BTreeMap<QueryKey, RequestRecovery>,
    terminals: &BTreeMap<QueryKey, QueryTerminalRecovery>,
    facts: &[Fact],
    snapshot: &mut Snapshot,
) -> Result<BTreeMap<u8, (Fact, native::DimensionTerminal)>> {
    let eligible = snapshot
        .definition
        .candidates
        .iter()
        .filter(|candidate| candidate.eligible)
        .map(|candidate| candidate.ordinal)
        .collect::<Vec<_>>();
    let mut dimensions = BTreeMap::new();
    for fact in facts {
        bound(fact, plan)?;
        let value: native::DimensionTerminal = codec::decode(&fact.bytes)?;
        let extra: (u8,u64,String,String,Option<u64>,Option<u32>,i64) = transaction.query_row(
            "SELECT dimension,plan_version,plan_sha256,outcome,last_query_terminal_version,selected_candidate_ordinal,pace_due FROM chain_post_close_macro_dimension_terminals WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(),fact.version], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)))
            .map_err(|_| storage("Macro dimension terminal"))?;
        require(
            value.version == 2
                && value.dimension
                    == u8::try_from(dimensions.len() + 1)
                        .map_err(|_| ChainPostCloseError::SchemaRejected)?
                && value.dimension <= 6
                && extra.0 == value.dimension
                && value.plan_version == plan_fact.version
                && extra.1 == plan_fact.version
                && value.plan_sha256 == plan_fact.digest
                && extra.2 == plan_fact.digest
                && value.eligible_candidates == eligible
                && extra.4 == value.last_query_terminal_version
                && extra.5 == value.selected_candidate
                && extra.6 == value.pace_due
                && Some(value.pace_due) == fact.time.checked_add(300_000),
        )?;
        let due = if value.dimension == 1 {
            snapshot
                .gateway_due()
                .map_err(|_| ChainPostCloseError::SchemaRejected)?
        } else {
            snapshot
                .dimensions
                .get(&(value.dimension - 1))
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .pace_due
        };
        require(fact.time >= due)?;
        let mut selected = None;
        let mut last = None;
        for candidate in &eligible {
            let query = QueryKey::Web {
                dimension: value.dimension,
                candidate: *candidate,
            };
            if selected.is_some() {
                require(!terminals.contains_key(&query) && !requests.contains_key(&query))?;
                continue;
            }
            let terminal = terminals
                .get(&query)
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            require(terminal.version <= fact.prior && terminal.time <= fact.time)?;
            last = Some(terminal.version);
            if !runner::web_lines(&QueryOutcome::Native(terminal.native.clone())).is_empty() {
                selected = Some(*candidate);
            }
        }
        require(
            selected == value.selected_candidate
                && last == value.last_query_terminal_version
                && extra.3
                    == if selected.is_some() {
                        "SelectedResearchOnly"
                    } else if eligible.is_empty() {
                        "NoEligibleProviders"
                    } else {
                        "Exhausted"
                    },
        )?;
        snapshot.dimensions.insert(
            value.dimension,
            Dimension {
                selected,
                pace_due: value.pace_due,
            },
        );
        require(
            dimensions
                .insert(value.dimension, (fact.clone(), value))
                .is_none(),
        )?;
    }
    // Validate admission ordering even when the current dimension has not yet
    // reached a terminal decision (including an unmatched in-flight begin).
    for (key, request) in requests {
        let QueryKey::Web {
            dimension,
            candidate,
        } = key
        else {
            continue;
        };
        let position = eligible
            .iter()
            .position(|ordinal| ordinal == candidate)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let due = if *dimension == 1 {
            snapshot
                .gateway_due()
                .map_err(|_| ChainPostCloseError::SchemaRejected)?
        } else {
            snapshot
                .dimensions
                .get(&(dimension - 1))
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .pace_due
        };
        require(request.fact.time >= due)?;
        for earlier in &eligible[..position] {
            let previous = terminals
                .get(&QueryKey::Web {
                    dimension: *dimension,
                    candidate: *earlier,
                })
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            require(
                previous.version <= request.fact.prior
                    && previous.time <= request.fact.time
                    && runner::web_lines(&QueryOutcome::Native(previous.native.clone())).is_empty(),
            )?;
        }
    }
    Ok(dimensions)
}
