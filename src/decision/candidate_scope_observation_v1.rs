//! Durable negative observations. No approval, investment decision, or Paper action.
use super::pushed_candidate_scope_v1::{
    self as source, CandidateScopeError, CapturedPushedCandidateScope,
};
use crate::database::candidate_scope_observation_schema_v1::{
    MAX_SCOPE_BYTES, POLICY, SLOT_MILLISECONDS,
};
use crate::database::global_schema_v1::candidate_v7::{
    candidate_catalog7_session, CandidateCatalog7Error, CandidateCatalog7ReadbackError,
    CandidateCatalog7TransactionError, VerifiedCatalog7,
};
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use chrono::{DateTime, Utc};
use diesel::sql_types::{BigInt, Binary, Text};
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};
use sha2::{Digest, Sha256};

const SCOPE_DOMAIN: &[u8] = b"stock_analysis.candidate_scope_observation.scope.v1\0";
const OCCURRENCE_DOMAIN: &[u8] = b"stock_analysis.candidate_scope_observation.occurrence.v1\0";
const REVISION: i64 = 1;
const MAX_SLOT: i64 = 253_402_300_770_000;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ObservationError {
    #[error(transparent)]
    Catalog(#[from] CandidateCatalog7Error),
    #[error(transparent)]
    Source(#[from] CandidateScopeError),
    #[error("observation storage read failed")]
    Sql(#[from] diesel::result::Error),
    #[error("observation storage types, bounds, or metadata differ")]
    InvalidRecord,
    #[error("observation clock is outside the fixed UTC slot contract")]
    Clock,
    #[error("observation immutable record differs")]
    Conflict,
    #[error("observation original physical authority differs")]
    Authority,
    #[error("observation rowid exhausted")]
    RowIdExhausted,
}

/// Historical bytes only. No deserialization or conversion to live qualification.
pub(crate) struct StoredCandidateScopeObservation {
    authority: DatabaseConnectionAuthority,
    row_id: i64,
    slot: i64,
    cutoff: DateTime<Utc>,
    digest: Vec<u8>,
    canonical: Vec<u8>,
    scope_id: String,
    occurrence_id: String,
}
impl StoredCandidateScopeObservation {
    pub(crate) fn occurrence_id(&self) -> &str {
        &self.occurrence_id
    }
    pub(crate) fn scope_id(&self) -> &str {
        &self.scope_id
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
    fn same_record(&self, other: &Self) -> bool {
        self.authority == other.authority
            && self.row_id == other.row_id
            && self.slot == other.slot
            && self.cutoff == other.cutoff
            && self.digest == other.digest
            && self.canonical == other.canonical
            && self.scope_id == other.scope_id
            && self.occurrence_id == other.occurrence_id
    }
}
struct ObservationAttempt {
    record: StoredCandidateScopeObservation,
    // Only a first append retains live capture. An existing key never gets one.
    captured: Option<CapturedPushedCandidateScope>,
}
fn slot_for(now: DateTime<Utc>) -> Result<i64, ObservationError> {
    let ms = now.timestamp_millis();
    if ms < 0 {
        return Err(ObservationError::Clock);
    }
    let slot = (ms / SLOT_MILLISECONDS) * SLOT_MILLISECONDS;
    if slot > MAX_SLOT {
        return Err(ObservationError::Clock);
    }
    Ok(slot)
}
fn scope_digest(bytes: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(SCOPE_DOMAIN);
    hash.update(bytes);
    hash.finalize().to_vec()
}
fn identities(slot: i64, digest: &[u8]) -> (String, String) {
    let mut hash = Sha256::new();
    hash.update(OCCURRENCE_DOMAIN);
    hash.update(POLICY.as_bytes());
    hash.update([0]);
    hash.update(slot.to_be_bytes());
    hash.update(REVISION.to_be_bytes());
    hash.update(digest);
    (
        format!("candidate-observation-scope-v1:{}", hex::encode(digest)),
        format!(
            "candidate-observation-occurrence-v1:{}",
            hex::encode(hash.finalize())
        ),
    )
}

#[derive(QueryableByName)]
struct Metadata {
    #[diesel(sql_type = BigInt)]
    row_id: i64,
    #[diesel(sql_type = BigInt)]
    seconds: i64,
    #[diesel(sql_type = BigInt)]
    nanos: i64,
    #[diesel(sql_type = BigInt)]
    invalid: i64,
}
#[derive(QueryableByName)]
struct Payload {
    #[diesel(sql_type = Binary)]
    scope_sha256: Vec<u8>,
    #[diesel(sql_type = Binary)]
    scope_canonical: Vec<u8>,
}
#[derive(QueryableByName)]
struct Maximum {
    #[diesel(sql_type = BigInt)]
    value: i64,
}

fn load_record(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog7<'_>,
    slot: i64,
) -> Result<Option<StoredCandidateScopeObservation>, ObservationError> {
    // Validate the callback-local instance before even the metadata SQL.
    proof.require_observation_instance(conn)?;
    let meta = diesel::sql_query("SELECT observation_row_id AS row_id,cutoff_unix_seconds AS seconds,cutoff_subsec_nanos AS nanos,CASE WHEN typeof(observation_row_id)!='integer' OR observation_row_id<=0 OR typeof(policy_id)!='text' OR length(CAST(policy_id AS BLOB))!=39 OR typeof(slot_start_unix_ms)!='integer' OR typeof(evaluation_revision)!='integer' OR evaluation_revision!=1 OR typeof(cutoff_unix_seconds)!='integer' OR typeof(cutoff_subsec_nanos)!='integer' OR cutoff_subsec_nanos<0 OR cutoff_subsec_nanos>999999999 OR typeof(scope_sha256)!='blob' OR length(scope_sha256)!=32 OR typeof(scope_canonical)!='blob' OR length(scope_canonical)<1 OR length(scope_canonical)>8388608 THEN 1 ELSE 0 END AS invalid FROM main.candidate_scope_observations_v1 WHERE policy_id=? AND slot_start_unix_ms=? AND evaluation_revision=1 LIMIT 2")
        .bind::<Text,_>(POLICY).bind::<BigInt,_>(slot).load::<Metadata>(conn)?;
    if meta.is_empty() {
        return Ok(None);
    }
    if meta.len() != 1 {
        return Err(ObservationError::InvalidRecord);
    }
    let m = &meta[0];
    if m.invalid != 0
        || slot < 0
        || slot > MAX_SLOT
        || slot % SLOT_MILLISECONDS != 0
        || m.seconds < slot / 1000
        || m.seconds >= slot / 1000 + 30
    {
        return Err(ObservationError::InvalidRecord);
    }
    let cutoff = DateTime::<Utc>::from_timestamp(
        m.seconds,
        u32::try_from(m.nanos).map_err(|_| ObservationError::InvalidRecord)?,
    )
    .ok_or(ObservationError::Clock)?;
    let payload = diesel::sql_query("SELECT scope_sha256,scope_canonical FROM main.candidate_scope_observations_v1 WHERE observation_row_id=? AND policy_id=? AND slot_start_unix_ms=? AND evaluation_revision=1")
        .bind::<BigInt,_>(m.row_id).bind::<Text,_>(POLICY).bind::<BigInt,_>(slot).get_result::<Payload>(conn)?;
    if payload.scope_canonical.len() > MAX_SCOPE_BYTES
        || scope_digest(&payload.scope_canonical) != payload.scope_sha256
    {
        return Err(ObservationError::InvalidRecord);
    }
    source::validate_stored_canonical(&payload.scope_canonical, cutoff)?;
    let (scope_id, occurrence_id) = identities(slot, &payload.scope_sha256);
    Ok(Some(StoredCandidateScopeObservation {
        authority: proof.connection_authority().clone(),
        row_id: m.row_id,
        slot,
        cutoff,
        digest: payload.scope_sha256,
        canonical: payload.scope_canonical,
        scope_id,
        occurrence_id,
    }))
}

/// One owner-selected UTC instant and fixed evaluation revision1.
pub(crate) fn observe_candidate_scope(
    db: &DatabaseManager,
) -> Result<StoredCandidateScopeObservation, CandidateCatalog7TransactionError<ObservationError>> {
    observe_at(db, Utc::now())
}
#[cfg(test)]
pub(crate) fn observe_candidate_scope_at_for_test(
    db: &DatabaseManager,
    now: DateTime<Utc>,
) -> Result<StoredCandidateScopeObservation, CandidateCatalog7TransactionError<ObservationError>> {
    if !db.has_isolated_p05_consumer_origin() {
        return Err(CandidateCatalog7TransactionError::BeforeCommit(
            CandidateCatalog7Error::Catalog7RequalificationRequired,
        ));
    }
    observe_at(db, now)
}
fn observe_at(
    db: &DatabaseManager,
    now: DateTime<Utc>,
) -> Result<StoredCandidateScopeObservation, CandidateCatalog7TransactionError<ObservationError>> {
    let mut session =
        candidate_catalog7_session(db).map_err(CandidateCatalog7TransactionError::BeforeCommit)?;
    let slot = slot_for(now).map_err(CandidateCatalog7TransactionError::Consumer)?;
    session.with_immediate_catalog7(
        |conn, _, proof| {
            if let Some(record) = load_record(conn, proof, slot)? {
                return Ok(ObservationAttempt { record, captured:None });
            }
            let captured = source::capture_catalog7_at(conn, proof, now)?;
            source::validate_stored_canonical(captured.canonical_bytes(), now)?;
            let max = diesel::sql_query("SELECT COALESCE(MAX(observation_row_id),0) AS value FROM main.candidate_scope_observations_v1").get_result::<Maximum>(conn)?.value;
            let row_id = max.checked_add(1).filter(|id| *id>0).ok_or(ObservationError::RowIdExhausted)?;
            let digest = scope_digest(captured.canonical_bytes());
            diesel::sql_query("INSERT INTO main.candidate_scope_observations_v1(observation_row_id,policy_id,slot_start_unix_ms,evaluation_revision,cutoff_unix_seconds,cutoff_subsec_nanos,scope_sha256,scope_canonical) VALUES(?,?,?,1,?,?,?,?)")
                .bind::<BigInt,_>(row_id).bind::<Text,_>(POLICY).bind::<BigInt,_>(slot).bind::<BigInt,_>(now.timestamp()).bind::<BigInt,_>(i64::from(now.timestamp_subsec_nanos())).bind::<Binary,_>(&digest).bind::<Binary,_>(captured.canonical_bytes()).execute(conn)?;
            let record = load_record(conn, proof, slot)?.ok_or(ObservationError::Conflict)?;
            if record.row_id != row_id || record.cutoff != now || record.canonical != captured.canonical_bytes() || record.digest != digest { return Err(ObservationError::Conflict); }
            Ok(ObservationAttempt { record, captured:Some(captured) })
        },
        |conn, authority, proof, attempt| {
            if authority != &attempt.record.authority || proof.connection_authority() != &attempt.record.authority { return Err(ObservationError::Authority); }
            let actual = load_record(conn, proof, slot)?.ok_or(ObservationError::Conflict)?;
            if !actual.same_record(&attempt.record) { return Err(ObservationError::Conflict); }
            if let Some(captured) = &attempt.captured { captured.verify_catalog7_unchanged(conn, proof)?; }
            Ok(())
        },
    ).map(|attempt| attempt.record)
}

/// A retained historical observation, never a claim of current source fitness.
pub(crate) fn read_candidate_scope_observation(
    db: &DatabaseManager,
    slot: i64,
) -> Result<Option<StoredCandidateScopeObservation>, CandidateCatalog7ReadbackError<ObservationError>>
{
    if slot < 0 || slot > MAX_SLOT || slot % SLOT_MILLISECONDS != 0 {
        return Err(CandidateCatalog7ReadbackError::Consumer(
            ObservationError::Clock,
        ));
    }
    let mut session = candidate_catalog7_session(db)
        .map_err(CandidateCatalog7ReadbackError::ObservationUnavailable)?;
    session.with_readonly_catalog7(
        |conn, proof| load_record(conn, proof, slot),
        |conn, proof, original| {
            let current = load_record(conn, proof, slot)?;
            match (original, current) {
                (None, None) => Ok(()),
                (Some(a), Some(b)) if a.same_record(&b) => Ok(()),
                _ => Err(ObservationError::Conflict),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_observation_identity_domains_literal_golden() {
        let digest = scope_digest(b"[]");
        assert_eq!(
            hex::encode(&digest),
            "a6dc8a17871efb6e5d21befa784865143c886df52899d04448bcfb475b9fe557"
        );
        assert_eq!(identities(0,&digest).1, "candidate-observation-occurrence-v1:5bb5e0cfb82d0c9bf07dbdc634a000f752813460690165fe9494eb86a14eb534");
        assert_ne!(identities(0, &digest).1, identities(30000, &digest).1);
        assert_ne!(digest, Sha256::digest(b"[]").to_vec());
    }
}
