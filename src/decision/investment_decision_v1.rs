//! Durable immutable negative evaluations. Historical evidence cannot approve trades.
use super::candidate_scope_observation_v1::{
    self as observation, ObservationAttempt, ObservationError,
};
use super::investment_decision_codec_v1::{self as codec, CodecError, Disposition};
use crate::database::candidate_scope_observation_schema_v1::{POLICY, SLOT_MILLISECONDS};
use crate::database::global_schema_v1::investment_v8::{
    investment_catalog8_session, InvestmentCatalog8Error, InvestmentCatalog8ReadbackError,
    InvestmentCatalog8TransactionError, VerifiedCatalog8,
};
use crate::database::investment_decision_schema_v1::STRATEGY;
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use chrono::{DateTime, Utc};
use diesel::sql_types::{BigInt, Binary, Text};
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};

const MAX_SLOT: i64 = 253_402_300_770_000;
#[derive(Debug, thiserror::Error)]
pub(crate) enum InvestmentRecordError {
    #[error(transparent)]
    Catalog(#[from] InvestmentCatalog8Error),
    #[error(transparent)]
    Observation(#[from] ObservationError),
    #[error("investment record codec rejected historical values: {0}")]
    Codec(String),
    #[error("investment record SQL failed")]
    Sql(#[from] diesel::result::Error),
    #[error("investment record stored type, key, bounds or canonical differs")]
    InvalidRecord,
    #[error("investment immutable key or original bytes differ")]
    Conflict,
    #[error("investment record rowid exhausted")]
    RowIdExhausted,
    #[error("investment occurrence clock is invalid")]
    Clock,
}
impl From<CodecError> for InvestmentRecordError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e.to_string())
    }
}
/// Independent identity namespace, constructed only by this complete durable owner.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvestmentDecisionId(String);
impl InvestmentDecisionId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
/// Non-Deserialize historical evidence; no conversion into an order or live facts.
pub(crate) struct RecordedInvestmentDecision {
    authority: DatabaseConnectionAuthority,
    row_id: i64,
    slot: i64,
    cutoff: DateTime<Utc>,
    observation_row_id: i64,
    id: InvestmentDecisionId,
    digest: Vec<u8>,
    canonical: Vec<u8>,
    disposition: Disposition,
    candidates: usize,
}
impl RecordedInvestmentDecision {
    pub(crate) fn id(&self) -> &InvestmentDecisionId {
        &self.id
    }
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn cutoff(&self) -> DateTime<Utc> {
        self.cutoff
    }
    pub(crate) fn slot_start_unix_ms(&self) -> i64 {
        self.slot
    }
    pub(crate) fn candidate_count(&self) -> usize {
        self.candidates
    }
    pub(crate) fn is_no_candidates(&self) -> bool {
        self.disposition == Disposition::NoCandidates
    }
    fn same_record(&self, b: &Self) -> bool {
        self.authority == b.authority
            && self.row_id == b.row_id
            && self.slot == b.slot
            && self.cutoff == b.cutoff
            && self.observation_row_id == b.observation_row_id
            && self.id == b.id
            && self.digest == b.digest
            && self.canonical == b.canonical
            && self.disposition == b.disposition
            && self.candidates == b.candidates
    }
}
struct Attempt {
    record: RecordedInvestmentDecision,
    scope: Option<ObservationAttempt>,
}
#[derive(QueryableByName)]
struct Metadata {
    #[diesel(sql_type=BigInt)]
    row_id: i64,
    #[diesel(sql_type=BigInt)]
    observation_row_id: i64,
    #[diesel(sql_type=BigInt)]
    seconds: i64,
    #[diesel(sql_type=BigInt)]
    nanos: i64,
    #[diesel(sql_type=BigInt)]
    invalid: i64,
}
#[derive(QueryableByName)]
struct Payload {
    #[diesel(sql_type=Binary)]
    decision_id: Vec<u8>,
    #[diesel(sql_type=Binary)]
    record_sha256: Vec<u8>,
    #[diesel(sql_type=Binary)]
    record_canonical: Vec<u8>,
}
#[derive(QueryableByName)]
struct Maximum {
    #[diesel(sql_type=BigInt)]
    value: i64,
}
fn valid_slot(slot: i64) -> bool {
    (0..=MAX_SLOT).contains(&slot) && slot % SLOT_MILLISECONDS == 0
}
fn slot_for(now: DateTime<Utc>) -> Result<i64, InvestmentRecordError> {
    let ms = now.timestamp_millis();
    let slot = ms / SLOT_MILLISECONDS * SLOT_MILLISECONDS;
    if ms < 0 || !valid_slot(slot) {
        Err(InvestmentRecordError::Clock)
    } else {
        Ok(slot)
    }
}
fn load(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog8<'_>,
    slot: i64,
) -> Result<Option<RecordedInvestmentDecision>, InvestmentRecordError> {
    proof.require_decision_instance(conn)?;
    if !valid_slot(slot) {
        return Err(InvestmentRecordError::Clock);
    }
    let meta=diesel::sql_query("SELECT decision_row_id AS row_id,observation_row_id,cutoff_unix_seconds AS seconds,cutoff_subsec_nanos AS nanos,CASE WHEN typeof(decision_row_id)!='integer' OR decision_row_id<=0 OR typeof(strategy_id)!='text' OR strategy_id!='intraday-pushed-research-v1' OR typeof(scope_policy_id)!='text' OR scope_policy_id!='intraday-unconsumed-pushed-row-top50-v1' OR typeof(slot_start_unix_ms)!='integer' OR typeof(evaluation_revision)!='integer' OR evaluation_revision!=1 OR typeof(observation_row_id)!='integer' OR observation_row_id<=0 OR typeof(cutoff_unix_seconds)!='integer' OR typeof(cutoff_subsec_nanos)!='integer' OR cutoff_subsec_nanos<0 OR cutoff_subsec_nanos>999999999 OR typeof(decision_id)!='text' OR length(CAST(decision_id AS BLOB))!=87 OR typeof(record_sha256)!='blob' OR length(record_sha256)!=32 OR typeof(record_canonical)!='blob' OR length(record_canonical)<1 OR length(record_canonical)>16777216 THEN 1 ELSE 0 END AS invalid FROM main.investment_decisions_v1 WHERE strategy_id=? AND scope_policy_id=? AND slot_start_unix_ms=? AND evaluation_revision=1 LIMIT 2")
        .bind::<Text,_>(STRATEGY).bind::<Text,_>(POLICY).bind::<BigInt,_>(slot).load::<Metadata>(conn)?;
    if meta.is_empty() {
        return Ok(None);
    }
    if meta.len() != 1 {
        return Err(InvestmentRecordError::InvalidRecord);
    }
    let m = &meta[0];
    if m.invalid != 0 || m.seconds < slot / 1000 || m.seconds >= slot / 1000 + 30 {
        return Err(InvestmentRecordError::InvalidRecord);
    }
    let cutoff = DateTime::from_timestamp(
        m.seconds,
        u32::try_from(m.nanos).map_err(|_| InvestmentRecordError::Clock)?,
    )
    .ok_or(InvestmentRecordError::Clock)?;
    let payload=diesel::sql_query("SELECT CAST(decision_id AS BLOB) AS decision_id,record_sha256,record_canonical FROM main.investment_decisions_v1 WHERE decision_row_id=? AND strategy_id=? AND scope_policy_id=? AND slot_start_unix_ms=? AND evaluation_revision=1")
        .bind::<BigInt,_>(m.row_id).bind::<Text,_>(STRATEGY).bind::<Text,_>(POLICY).bind::<BigInt,_>(slot).get_result::<Payload>(conn)?;
    if payload.record_canonical.len()
        > crate::database::investment_decision_schema_v1::MAX_RECORD_BYTES
        || codec::digest(&payload.record_canonical) != payload.record_sha256
        || codec::id(&payload.record_canonical).as_bytes() != payload.decision_id
    {
        return Err(InvestmentRecordError::InvalidRecord);
    }
    let observation = observation::load_catalog8_scope(conn, proof, slot)?
        .ok_or(InvestmentRecordError::Conflict)?;
    if observation.row_id() != m.observation_row_id || observation.cutoff() != cutoff {
        return Err(InvestmentRecordError::Conflict);
    }
    let record = codec::decode(&payload.record_canonical, &observation)?;
    if record.observation_row_id() != m.observation_row_id {
        return Err(InvestmentRecordError::Conflict);
    }
    Ok(Some(RecordedInvestmentDecision {
        authority: proof.connection_authority().clone(),
        row_id: m.row_id,
        slot,
        cutoff,
        observation_row_id: m.observation_row_id,
        id: InvestmentDecisionId(
            String::from_utf8(payload.decision_id)
                .map_err(|_| InvestmentRecordError::InvalidRecord)?,
        ),
        digest: payload.record_sha256,
        canonical: payload.record_canonical,
        disposition: record.disposition().clone(),
        candidates: record.candidate_count(),
    }))
}
fn append(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog8<'_>,
    scope: &observation::StoredCandidateScopeObservation,
    bytes: &[u8],
) -> Result<RecordedInvestmentDecision, InvestmentRecordError> {
    proof.require_decision_instance(conn)?;
    // This check is also the explicit same-key changed-byte collision boundary.
    if let Some(existing) = load(conn, proof, scope.slot_start_unix_ms())? {
        if existing.canonical_bytes() == bytes {
            return Ok(existing);
        }
        return Err(InvestmentRecordError::Conflict);
    }
    codec::decode(bytes, scope)?;
    let row_id = diesel::sql_query(
        "SELECT COALESCE(MAX(decision_row_id),0) AS value FROM main.investment_decisions_v1",
    )
    .get_result::<Maximum>(conn)?
    .value
    .checked_add(1)
    .filter(|x| *x > 0)
    .ok_or(InvestmentRecordError::RowIdExhausted)?;
    let id = codec::id(bytes);
    let digest = codec::digest(bytes);
    diesel::sql_query("INSERT INTO main.investment_decisions_v1(decision_row_id,strategy_id,scope_policy_id,slot_start_unix_ms,evaluation_revision,observation_row_id,cutoff_unix_seconds,cutoff_subsec_nanos,decision_id,record_sha256,record_canonical) VALUES(?,?,?,?,1,?,?,?,?,?,?)")
        .bind::<BigInt,_>(row_id).bind::<Text,_>(STRATEGY).bind::<Text,_>(POLICY).bind::<BigInt,_>(scope.slot_start_unix_ms()).bind::<BigInt,_>(scope.row_id()).bind::<BigInt,_>(scope.cutoff().timestamp()).bind::<BigInt,_>(i64::from(scope.cutoff().timestamp_subsec_nanos())).bind::<Text,_>(&id).bind::<Binary,_>(&digest).bind::<Binary,_>(bytes).execute(conn)?;
    let stored =
        load(conn, proof, scope.slot_start_unix_ms())?.ok_or(InvestmentRecordError::Conflict)?;
    if stored.row_id != row_id
        || stored.canonical != bytes
        || stored.digest != digest
        || stored.id.as_str() != id
    {
        return Err(InvestmentRecordError::Conflict);
    }
    Ok(stored)
}
pub(crate) fn record_investment_decision(
    db: &DatabaseManager,
    config: &crate::config::LiveVetoConfig,
) -> Result<RecordedInvestmentDecision, InvestmentCatalog8TransactionError<InvestmentRecordError>> {
    record_at(db, config, Utc::now())
}
fn record_at(
    db: &DatabaseManager,
    config: &crate::config::LiveVetoConfig,
    now: DateTime<Utc>,
) -> Result<RecordedInvestmentDecision, InvestmentCatalog8TransactionError<InvestmentRecordError>> {
    let mut session = investment_catalog8_session(db)
        .map_err(InvestmentCatalog8TransactionError::BeforeCommit)?;
    let slot = slot_for(now).map_err(InvestmentCatalog8TransactionError::Consumer)?;
    session
        .with_immediate_catalog8(
            |conn, _, proof| {
                if let Some(record) = load(conn, proof, slot)? {
                    return Ok(Attempt {
                        record,
                        scope: None,
                    });
                }
                let scope = observation::observe_catalog8_scope(conn, proof, now)?;
                let bytes = codec::build(&scope.record, config)?;
                let record = append(conn, proof, &scope.record, &bytes)?;
                Ok(Attempt {
                    record,
                    scope: Some(scope),
                })
            },
            |conn, authority, proof, attempt| {
                if authority != &attempt.record.authority
                    || proof.connection_authority() != &attempt.record.authority
                {
                    return Err(InvestmentRecordError::Conflict);
                }
                let stored = load(conn, proof, slot)?.ok_or(InvestmentRecordError::Conflict)?;
                if !stored.same_record(&attempt.record) {
                    return Err(InvestmentRecordError::Conflict);
                }
                if let Some(scope) = &attempt.scope {
                    observation::verify_catalog8_scope(conn, proof, scope)?;
                }
                Ok(())
            },
        )
        .map(|a| a.record)
}
#[cfg(test)]
pub(crate) fn record_investment_decision_at_for_test(
    db: &DatabaseManager,
    config: &crate::config::LiveVetoConfig,
    now: DateTime<Utc>,
) -> Result<RecordedInvestmentDecision, InvestmentCatalog8TransactionError<InvestmentRecordError>> {
    if !db.has_isolated_p05_consumer_origin() {
        return Err(InvestmentCatalog8TransactionError::BeforeCommit(
            InvestmentCatalog8Error::Catalog8RequalificationRequired,
        ));
    }
    record_at(db, config, now)
}
pub(crate) fn read_investment_decision(
    db: &DatabaseManager,
    slot: i64,
) -> Result<
    Option<RecordedInvestmentDecision>,
    InvestmentCatalog8ReadbackError<InvestmentRecordError>,
> {
    if !valid_slot(slot) {
        return Err(InvestmentCatalog8ReadbackError::Consumer(
            InvestmentRecordError::Clock,
        ));
    }
    let mut session = investment_catalog8_session(db)
        .map_err(InvestmentCatalog8ReadbackError::ObservationUnavailable)?;
    session.with_readonly_catalog8(
        |c, p| load(c, p, slot),
        |c, p, original| {
            let current = load(c, p, slot)?;
            match (original, current) {
                (None, None) => Ok(()),
                (Some(a), Some(b)) if a.same_record(&b) => Ok(()),
                _ => Err(InvestmentRecordError::Conflict),
            }
        },
    )
}
#[cfg(test)]
pub(crate) fn changed_bytes_for_test(
    db: &DatabaseManager,
    slot: i64,
    config: &crate::config::LiveVetoConfig,
) -> Result<(), InvestmentCatalog8TransactionError<InvestmentRecordError>> {
    let mut session = investment_catalog8_session(db)
        .map_err(InvestmentCatalog8TransactionError::BeforeCommit)?;
    session.with_immediate_catalog8(
        |conn, _, p| {
            let o = observation::load_catalog8_scope(conn, p, slot)?
                .ok_or(InvestmentRecordError::Conflict)?;
            let bytes = codec::build(&o, config)?;
            append(conn, p, &o, &bytes).map(|_| ())
        },
        |_, _, _, _| Ok(()),
    )
}
