//! V13 Models/Search/Report effect journal on the parent's SQLite handle.
//!
//! Every renderer effect (model call, topic search, availability read, local
//! clock read) is recorded as a begin row before it runs and a result row after
//! it returns; the assembled preparation artifact is sealed once as the stage
//! final. Reopen replays stored outcomes in ordinal order and never re-issues a
//! remote call for an ordinal that already has a result.
use super::*;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::search_service::SearchResult;

type Result<T> = std::result::Result<T, ChainPostCloseError>;

pub(super) const TABLES: [&str; 3] = [
    "chain_post_close_models_effect_begins",
    "chain_post_close_models_effect_results",
    "chain_post_close_models_stage_finals",
];

fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(ChainPostCloseError::SchemaRejected)
    }
}

/// Fixed request material of one renderer effect. Equality is byte equality of
/// the canonical encoding, so a replay with any drift is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Request {
    ModelAvailable,
    SearchAvailable,
    LocalNow,
    Model {
        stage: String,
        concept: Option<String>,
        prompt: String,
        system: String,
        mode: String,
    },
    Search {
        stage: String,
        query: String,
        limit: u64,
    },
}

impl Request {
    fn kind(&self) -> &'static str {
        match self {
            Request::ModelAvailable => "ModelAvailable",
            Request::SearchAvailable => "SearchAvailable",
            Request::LocalNow => "LocalNow",
            Request::Model { .. } => "Model",
            Request::Search { .. } => "Search",
        }
    }
    fn model_stage(&self) -> Option<&str> {
        match self {
            Request::Model { stage, .. } => Some(stage),
            _ => None,
        }
    }
    fn concept(&self) -> Option<&str> {
        match self {
            Request::Model { concept, .. } => concept.as_deref(),
            _ => None,
        }
    }
    fn search_stage(&self) -> Option<&str> {
        match self {
            Request::Search { stage, .. } => Some(stage),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Outcome {
    Available(bool),
    LocalNow(String),
    Returned(String),
    Failed(String),
    SearchReturned(Vec<SearchResult>),
}

impl Outcome {
    fn column(&self) -> &'static str {
        match self {
            Outcome::Failed(_) => "Failed",
            _ => "Returned",
        }
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| ChainPostCloseError::SchemaRejected)
}

fn decode<T: Serialize + for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    let value: T =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    // Only the exact canonical bytes are accepted back.
    require(encode(&value)? == bytes)?;
    Ok(value)
}

pub(super) struct Begun {
    pub(super) ordinal: u64,
    version: u64,
    request_sha256: String,
    owner: String,
    generation: u64,
}

pub(super) struct Recovery {
    /// Effects in ordinal order; only the last may lack an outcome.
    pub(super) effects: Vec<(Request, Option<Outcome>)>,
    pub(super) artifact: Option<Vec<u8>>,
}

impl Recovery {
    pub(super) fn begun_unconfirmed(&self) -> bool {
        self.effects
            .last()
            .is_some_and(|(_, outcome)| outcome.is_none())
    }
}

struct MacroParent {
    version: u64,
    sha256: String,
}

fn validate_identity(recovery: &RunRecovery, lease: &RunLease) -> Result<()> {
    require(
        recovery.context.run_id() == &lease.run_id
            && recovery.input.encode()? == lease.input.encode()?
            && recovery.generation == lease.generation
            && recovery.head == lease.head,
    )
}

fn advance(transaction: &Transaction<'_>, lease: &mut RunLease, now: UtcMicros) -> Result<u64> {
    let previous = lease.head;
    let next = previous
        .checked_add(1)
        .ok_or_else(|| storage("models head"))?;
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
        .map_err(|_| storage("models cas"))?;
    if changed != 1 {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    lease.head = next;
    Ok(previous)
}

struct BeginRow {
    ordinal: u64,
    version: u64,
    request_sha256: String,
    request: Request,
    owner: String,
    generation: u64,
    recorded_at: i64,
}

struct ResultRow {
    ordinal: u64,
    begin_version: u64,
    version: u64,
    request_sha256: String,
    outcome: Outcome,
    recorded_at: i64,
}

fn begins(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<Vec<BeginRow>> {
    let mut statement = transaction
        .prepare(
            "SELECT effect_ordinal,run_version,request_sha256,request_bytes,lease_owner,\
             lease_generation,recorded_at,effect_kind,model_stage,concept,search_stage,\
             request_length,run_id,run_context_sha256,input_sha256 \
             FROM chain_post_close_models_effect_begins WHERE intent_id=?1 ORDER BY effect_ordinal",
        )
        .map_err(|_| storage("models begins"))?;
    let rows = statement
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
            ))
        })
        .map_err(|_| storage("models begins"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("models begins"))?;
    let input_sha = raw_digest(&run.input.encode()?);
    let mut out = Vec::with_capacity(rows.len());
    for (index, row) in rows.into_iter().enumerate() {
        let (
            ordinal,
            version,
            request_sha256,
            bytes,
            owner,
            generation,
            recorded_at,
            kind,
            model_stage,
            concept,
            search_stage,
            length,
            run_id,
            context_sha,
            input,
        ) = row;
        let request: Request = decode(&bytes)?;
        require(
            ordinal == index as u64 + 1
                && raw_digest(&bytes).as_str() == request_sha256
                && usize::try_from(length).ok() == Some(bytes.len())
                && request.kind() == kind
                && request.model_stage() == model_stage.as_deref()
                && request.concept() == concept.as_deref()
                && request.search_stage() == search_stage.as_deref()
                && run_id == run.context.run_id().as_str()
                && context_sha == run.context.canonical_sha256().as_str()
                && input == input_sha.as_str()
                && version <= run.head
                && recorded_at <= run.updated_at
                && generation <= run.generation
                && (generation != run.generation || owner == run.owner),
        )?;
        out.push(BeginRow {
            ordinal,
            version,
            request_sha256,
            request,
            owner,
            generation,
            recorded_at,
        });
    }
    Ok(out)
}

fn results(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<Vec<ResultRow>> {
    let mut statement = transaction
        .prepare(
            "SELECT effect_ordinal,begin_run_version,run_version,request_sha256,outcome,\
             result_bytes,result_sha256,result_length,recorded_at \
             FROM chain_post_close_models_effect_results WHERE intent_id=?1 ORDER BY effect_ordinal",
        )
        .map_err(|_| storage("models results"))?;
    let rows = statement
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .map_err(|_| storage("models results"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("models results"))?;
    let mut out = Vec::with_capacity(rows.len());
    for (index, row) in rows.into_iter().enumerate() {
        let (
            ordinal,
            begin_version,
            version,
            request_sha256,
            column,
            bytes,
            sha,
            length,
            recorded_at,
        ) = row;
        let outcome: Outcome = decode(&bytes)?;
        require(
            ordinal == index as u64 + 1
                && raw_digest(&bytes).as_str() == sha
                && usize::try_from(length).ok() == Some(bytes.len())
                && outcome.column() == column
                && begin_version < version
                && version <= run.head
                && recorded_at <= run.updated_at,
        )?;
        out.push(ResultRow {
            ordinal,
            begin_version,
            version,
            request_sha256,
            outcome,
            recorded_at,
        });
    }
    Ok(out)
}

fn stage_final(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<Option<(u64, u64, Vec<u8>)>> {
    let row = transaction
        .query_row(
            "SELECT last_effect_ordinal,last_result_version,artifact_bytes,artifact_sha256,\
             artifact_length,report_bytes,report_sha256,report_length,run_version,recorded_at \
             FROM chain_post_close_models_stage_finals WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, u64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("models final"))?;
    let Some((
        last_ordinal,
        last_version,
        artifact,
        artifact_sha,
        artifact_length,
        report,
        report_sha,
        report_length,
        version,
        recorded_at,
    )) = row
    else {
        return Ok(None);
    };
    let prepared =
        crate::pipeline::chain_analysis::preparation::PreparedChainAnalysis::from_artifact_bytes(
            &artifact,
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    require(
        raw_digest(&artifact).as_str() == artifact_sha
            && usize::try_from(artifact_length).ok() == Some(artifact.len())
            && raw_digest(&report).as_str() == report_sha
            && usize::try_from(report_length).ok() == Some(report.len())
            && prepared.report().as_bytes() == report.as_slice()
            && last_version < version
            && version <= run.head
            && recorded_at <= run.updated_at,
    )?;
    Ok(Some((last_ordinal, last_version, artifact)))
}

/// Cross-checks begins, results and the final into one ordered recovery.
fn assemble(
    begins: Vec<BeginRow>,
    results: Vec<ResultRow>,
    final_: Option<(u64, u64, Vec<u8>)>,
) -> Result<Recovery> {
    require(results.len() <= begins.len() && begins.len() - results.len() <= 1)?;
    let mut effects = Vec::with_capacity(begins.len());
    let mut last_result_version = None;
    for (index, begin) in begins.into_iter().enumerate() {
        let outcome = match results.get(index) {
            Some(result) => {
                require(
                    result.ordinal == begin.ordinal
                        && result.begin_version == begin.version
                        && result.request_sha256 == begin.request_sha256
                        && result.recorded_at >= begin.recorded_at,
                )?;
                last_result_version = Some(result.version);
                Some(result.outcome.clone())
            }
            None => None,
        };
        let _ = (&begin.owner, begin.generation);
        effects.push((begin.request, outcome));
    }
    let artifact = match final_ {
        Some((last_ordinal, last_version, artifact)) => {
            require(
                effects.len() as u64 == last_ordinal
                    && last_result_version == Some(last_version)
                    && effects.last().is_some_and(|(_, outcome)| outcome.is_some()),
            )?;
            Some(artifact)
        }
        None => None,
    };
    Ok(Recovery { effects, artifact })
}

/// Fact validation entry used by whole-store inspection at layout 13.
pub(super) fn validate_facts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
) -> Result<()> {
    let begins = begins(transaction, intent, run)?;
    let results = results(transaction, intent, run)?;
    let final_ = stage_final(transaction, intent, run)?;
    assemble(begins, results, final_).map(|_| ())
}

impl LocalChainPostClose<'_> {
    fn models_parent(
        transaction: &Transaction<'_>,
        lease: &RunLease,
        now: UtcMicros,
    ) -> Result<(RunRecovery, MacroParent)> {
        schema::verify_runtime_layout_version(transaction, 13)?;
        let recovery = inspect_run_on_at_layout(transaction, &lease.intent_id, 13)?;
        check_lease(transaction, lease, now)?;
        validate_identity(&recovery, lease)?;
        let (version, sha256): (u64, String) = transaction
            .query_row(
                "SELECT run_version,sha256 FROM chain_post_close_macro_stage_finals WHERE intent_id=?1",
                [lease.intent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| storage("models macro parent"))?
            .ok_or(ChainPostCloseError::MacroNotStarted)?;
        require(version <= recovery.head)?;
        Ok((recovery, MacroParent { version, sha256 }))
    }

    pub(super) fn load_models(&mut self, lease: &RunLease, now: UtcMicros) -> Result<Recovery> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        let (run, _) = Self::models_parent(&transaction, lease, now)?;
        let begins = begins(&transaction, &lease.intent_id, &run)?;
        let results = results(&transaction, &lease.intent_id, &run)?;
        let final_ = stage_final(&transaction, &lease.intent_id, &run)?;
        let recovery = assemble(begins, results, final_)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(recovery)
    }

    pub(super) fn begin_models_effect(
        &mut self,
        mut lease: RunLease,
        ordinal: u64,
        request: &Request,
        now: UtcMicros,
    ) -> Result<(RunLease, Begun)> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, parent) = Self::models_parent(&transaction, &lease, now)?;
        let bytes = encode(request)?;
        let digest = raw_digest(&bytes);
        let prior = advance(&transaction, &mut lease, now)?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_models_effect_begins(\
                 intent_id,effect_ordinal,effect_kind,model_stage,concept,search_stage,\
                 request_codec_version,request_bytes,request_length,request_sha256,\
                 macro_final_version,macro_final_sha256,run_id,run_context_sha256,input_sha256,\
                 lease_owner,lease_generation,prior_head_version,run_version,recorded_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                params![
                    lease.intent_id.as_str(),
                    ordinal,
                    request.kind(),
                    request.model_stage(),
                    request.concept(),
                    request.search_stage(),
                    &bytes,
                    bytes.len(),
                    digest.as_str(),
                    parent.version,
                    &parent.sha256,
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
            .map_err(|_| storage("models begin insert"))?;
        transaction.commit().map_err(|_| storage("commit"))?;
        let begun = Begun {
            ordinal,
            version: lease.head,
            request_sha256: digest.as_str().to_owned(),
            owner: lease.owner.as_str().to_owned(),
            generation: lease.generation,
        };
        Ok((lease, begun))
    }

    pub(super) fn record_models_effect_result(
        &mut self,
        mut lease: RunLease,
        begun: Begun,
        outcome: &Outcome,
        now: UtcMicros,
    ) -> Result<RunLease> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, _) = Self::models_parent(&transaction, &lease, now)?;
        require(begun.owner == lease.owner.as_str() && begun.generation == lease.generation)?;
        let bytes = encode(outcome)?;
        let digest = raw_digest(&bytes);
        let prior = advance(&transaction, &mut lease, now)?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_models_effect_results(\
                 intent_id,effect_ordinal,begin_run_version,request_sha256,outcome,\
                 result_codec_version,result_bytes,result_length,result_sha256,\
                 run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,\
                 prior_head_version,run_version,recorded_at) \
                 VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![
                    lease.intent_id.as_str(),
                    begun.ordinal,
                    begun.version,
                    &begun.request_sha256,
                    outcome.column(),
                    &bytes,
                    bytes.len(),
                    digest.as_str(),
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
            .map_err(|_| storage("models result insert"))?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(lease)
    }

    /// Seals the assembled artifact once. A second call with byte-identical
    /// material is a no-op; any other material is refused.
    pub(super) fn finalize_models_stage(
        &mut self,
        mut lease: RunLease,
        artifact: &[u8],
        report: &[u8],
        now: UtcMicros,
    ) -> Result<RunLease> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let (run, _) = Self::models_parent(&transaction, &lease, now)?;
        if let Some((_, _, existing)) = stage_final(&transaction, &lease.intent_id, &run)? {
            require(existing == artifact)?;
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(lease);
        }
        let results = results(&transaction, &lease.intent_id, &run)?;
        let last = results.last().ok_or(ChainPostCloseError::SchemaRejected)?;
        let (last_ordinal, last_version) = (last.ordinal, last.version);
        let prior = advance(&transaction, &mut lease, now)?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_models_stage_finals(\
                 intent_id,last_effect_ordinal,last_result_version,artifact_codec_version,\
                 artifact_bytes,artifact_length,artifact_sha256,report_bytes,report_length,\
                 report_sha256,run_id,run_context_sha256,input_sha256,lease_owner,\
                 lease_generation,prior_head_version,run_version,recorded_at) \
                 VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![
                    lease.intent_id.as_str(),
                    last_ordinal,
                    last_version,
                    artifact,
                    artifact.len(),
                    raw_digest(artifact).as_str(),
                    report,
                    report.len(),
                    raw_digest(report).as_str(),
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
            .map_err(|_| storage("models final insert"))?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(lease)
    }
}
