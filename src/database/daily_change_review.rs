//! BR-171 persisted review. CLI decisions can only refer to stored candidates.
use chrono::{DateTime, Utc};
use diesel::{
    sql_types::{BigInt, Text},
    Connection, RunQueryDsl, SqliteConnection,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::daily_change_confirmation::{self as legacy, DailyChangeConfirmationQuery};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSnapshot {
    pub schema_version: u32,
    pub discovery_contract: String,
    pub rule_version: String,
    pub instrument: crate::market_domain::InstrumentId,
    pub query: DailyChangeConfirmationQuery,
    pub fact_payload: serde_json::Value,
    pub raw_evidence: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateReview {
    pub candidate_id: String,
    pub revision: i64,
    pub snapshot: ReviewSnapshot,
    pub discovered_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub evidence_token: String,
    pub status: String,
    pub observations: Vec<ReviewSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Confirm,
    Reject,
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("daily_change_review_unavailable_v1")]
    Unavailable,
    #[error("daily_change_review_not_found_v1")]
    NotFound,
    #[error("daily_change_review_invalid_token_v1")]
    InvalidToken,
    #[error("daily_change_review_superseded_v1")]
    Superseded,
    #[error("daily_change_review_expired_v1")]
    Expired,
    #[error("daily_change_review_conflict_v1")]
    Conflict,
    #[error("daily_change_review_already_confirmed_v1")]
    AlreadyConfirmed,
    #[error("daily_change_review_scope_mismatch_v1")]
    ScopeMismatch,
    #[error("daily_change_review_output_v1: {0}")]
    Output(#[from] std::io::Error),
    #[error("daily_change_review_audit_failure_v1: {0}")]
    Audit(String),
    #[error("daily_change_review_database_v1: {0}")]
    Database(#[from] diesel::result::Error),
}
pub type ReviewResult<T> = Result<T, ReviewError>;

/// DB-only action contract shared by the CLI and embedded operator clients.
pub enum ReviewAction<'a> {
    Review,
    Decide {
        token: &'a str,
        decision: ReviewDecision,
        operator: &'a str,
        reason: &'a str,
    },
    Renew {
        token: &'a str,
    },
}
pub struct ReviewSelection<'a> {
    pub candidate_id: &'a str,
    pub code: Option<&'a str>,
    pub previous_date: Option<chrono::NaiveDate>,
    pub current_date: Option<chrono::NaiveDate>,
}

pub fn run_review_action_on_conn(
    conn: &mut SqliteConnection,
    selection: ReviewSelection<'_>,
    action: ReviewAction<'_>,
    now: DateTime<Utc>,
    output: &mut impl std::io::Write,
) -> ReviewResult<CandidateReview> {
    let original = review_on_conn(conn, selection.candidate_id, now)?;
    let query = &original.snapshot.query;
    if selection.code.is_some_and(|code| code != query.code)
        || selection
            .previous_date
            .is_some_and(|date| date != query.previous_date)
        || selection
            .current_date
            .is_some_and(|date| date != query.current_date)
    {
        return Err(ReviewError::ScopeMismatch);
    }
    // A failed preview/flush must never become a durable decision. The owner
    // rechecks token/revision inside its write transaction after this output.
    writeln!(output, "{}", json(&original)?)?;
    output.flush()?;
    let receipt = match action {
        ReviewAction::Review => return Ok(original),
        ReviewAction::Decide {
            token,
            decision,
            operator,
            reason,
        } => decide_on_conn(
            conn,
            selection.candidate_id,
            token,
            decision,
            operator,
            reason,
            now,
        )?,
        ReviewAction::Renew { token } => renew_on_conn(conn, selection.candidate_id, token, now)?,
    };
    // An output error here does not undo the committed fact; a retry recovers
    // that exact terminal/revision without refreshing TTL or writing an alias.
    writeln!(output, "{}", json(&receipt)?)?;
    output.flush()?;
    Ok(receipt)
}

impl From<legacy::DailyChangeConfirmationError> for ReviewError {
    fn from(error: legacy::DailyChangeConfirmationError) -> Self {
        Self::Audit(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionFact {
    candidate_id: String,
    token: String,
    decision: ReviewDecision,
    operator: String,
    reason: String,
    decided_at: DateTime<Utc>,
    confirmation: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Event {
    Candidate {
        review: CandidateReview,
        scope: String,
        stable_fact: String,
        renewal_of: Option<String>,
    },
    Observation {
        candidate_id: String,
        snapshot: ReviewSnapshot,
        observed_at: DateTime<Utc>,
    },
    Decision {
        fact: DecisionFact,
    },
}

#[derive(diesel::QueryableByName)]
struct Row {
    #[diesel(sql_type=BigInt)]
    seq: i64,
    #[diesel(sql_type=BigInt)]
    schema_version: i64,
    #[diesel(sql_type=Text)]
    command_id: String,
    #[diesel(sql_type=Text)]
    scope_key: String,
    #[diesel(sql_type=Text)]
    candidate_id: String,
    #[diesel(sql_type=BigInt)]
    revision: i64,
    #[diesel(sql_type=Text)]
    kind: String,
    #[diesel(sql_type=Text)]
    payload: String,
    #[diesel(sql_type=Text)]
    previous_hash: String,
    #[diesel(sql_type=Text)]
    record_hash: String,
}

struct CandidateState {
    review: CandidateReview,
    scope: String,
    stable_fact: String,
    decision: Option<DecisionFact>,
    observations: Vec<ReviewSnapshot>,
    renewal_of: Option<String>,
}
#[derive(Default)]
struct State {
    candidates: BTreeMap<String, CandidateState>,
    latest: BTreeMap<String, String>,
    seq: i64,
    head: String,
}

fn audit(message: impl Into<String>) -> ReviewError {
    ReviewError::Audit(message.into())
}
fn require(condition: bool, message: &str) -> ReviewResult<()> {
    if condition {
        Ok(())
    } else {
        Err(audit(message))
    }
}
fn json(value: &impl Serialize) -> ReviewResult<String> {
    serde_json::to_string(value).map_err(|e| audit(e.to_string()))
}
fn hash(value: &impl Serialize) -> ReviewResult<String> {
    let mut h = Sha256::new();
    h.update(b"stock_analysis.daily_change_review.v1\0");
    h.update(json(value)?);
    Ok(hex::encode(h.finalize()))
}
fn identities(snapshot: &ReviewSnapshot) -> ReviewResult<(String, String)> {
    require(
        snapshot.schema_version == 1
            && snapshot.discovery_contract == "outcome-provider-sequence-v1",
        "unknown discovery schema",
    )?;
    require(
        snapshot.instrument.code() == snapshot.query.code
            && snapshot.instrument.asset_class() == crate::market_domain::AssetClass::Equity,
        "instrument mismatch",
    )?;
    let known_rule = snapshot.rule_version == "br171-close-change-v1";
    #[cfg(test)]
    let known_rule = known_rule || snapshot.rule_version == "TEST_CODE_rule_revision2";
    require(known_rule, "unknown rule version")?;
    let stable_v2 = legacy::daily_change_review_token_v2(&snapshot.query).map_err(audit)?;
    Ok((
        hash(&(
            "scope",
            &snapshot.instrument,
            snapshot.query.previous_date,
            snapshot.query.current_date,
        ))?,
        hash(&(
            "fact",
            &snapshot.instrument,
            &snapshot.rule_version,
            stable_v2,
            &snapshot.fact_payload,
        ))?,
    ))
}
fn candidate(
    snapshot: &ReviewSnapshot,
    revision: i64,
    now: DateTime<Utc>,
) -> ReviewResult<CandidateReview> {
    let (scope, stable) = identities(snapshot)?;
    let expires_at = now
        .checked_add_signed(chrono::Duration::days(7))
        .ok_or_else(|| audit("expiry overflow"))?;
    let id = format!(
        "br171_review_{}",
        hash(&(
            "candidate",
            &scope,
            revision,
            &stable,
            hash(snapshot)?,
            now,
            expires_at
        ))?
    );
    let token = hash(&("token", &id, revision, &stable, hash(snapshot)?, expires_at))?;
    Ok(CandidateReview {
        candidate_id: id,
        revision,
        snapshot: snapshot.clone(),
        discovered_at: now,
        expires_at,
        evidence_token: token,
        status: "Pending".into(),
        observations: Vec::new(),
    })
}
fn row_hash(row: &Row) -> ReviewResult<String> {
    hash(&(
        "event",
        row.seq,
        row.schema_version,
        &row.command_id,
        &row.scope_key,
        &row.candidate_id,
        row.revision,
        &row.kind,
        &row.payload,
        &row.previous_hash,
    ))
}
fn acquisition(snapshot: &ReviewSnapshot) -> ReviewResult<String> {
    hash(&(
        "acquisition",
        &snapshot.query.daily_batch_id,
        &snapshot.query.lifecycle_batch_id,
        // A provider may reuse its batch ID across distinct qualified requests.
        // Their immutable observations differ, while the stable fact/token do
        // not. Exactly replaying this full acquisition remains idempotent.
        hash(snapshot)?,
    ))
}

fn load(conn: &mut SqliteConnection) -> ReviewResult<State> {
    if !super::daily_change_review_schema_v1::is_present(conn)? {
        return Err(ReviewError::Unavailable);
    }
    let rows:Vec<Row> = diesel::sql_query("SELECT seq,schema_version,command_id,scope_key,candidate_id,revision,kind,payload,previous_hash,record_hash FROM daily_change_review_event ORDER BY seq").load(conn)?;
    let mut state = State::default();
    for row in rows {
        require(
            row.seq == state.seq + 1
                && row.schema_version == 1
                && row.previous_hash == state.head
                && row.record_hash == row_hash(&row)?,
            "review chain mismatch",
        )?;
        let event: Event = serde_json::from_str(&row.payload).map_err(|e| audit(e.to_string()))?;
        require(json(&event)? == row.payload, "noncanonical review event")?;
        match event {
            Event::Candidate {
                review,
                scope,
                stable_fact,
                renewal_of,
            } => {
                let ids = identities(&review.snapshot)?;
                let previous_revision = state
                    .latest
                    .get(&scope)
                    .map(|id| state.candidates[id].review.revision)
                    .unwrap_or(0);
                require(
                    row.kind == "Candidate"
                        && scope == ids.0
                        && stable_fact == ids.1
                        && scope == row.scope_key
                        && review.candidate_id == row.candidate_id
                        && review.revision == row.revision
                        && review.revision == previous_revision + 1
                        && review
                            == candidate(&review.snapshot, review.revision, review.discovered_at)?
                        && row.command_id == format!("candidate:{}", review.candidate_id),
                    "candidate identity mismatch",
                )?;
                if let Some(previous_id) = state.latest.get(&scope) {
                    let previous = &state.candidates[previous_id];
                    require(
                        review.discovered_at >= previous.review.discovered_at,
                        "revision clock regressed",
                    )?;
                    if stable_fact == previous.stable_fact {
                        require(
                            renewal_of.as_ref() == Some(previous_id)
                                && previous.decision.is_none()
                                && review.discovered_at >= previous.review.expires_at
                                && review.snapshot == previous.review.snapshot,
                            "invalid renewal",
                        )?;
                    } else {
                        require(renewal_of.is_none(), "fact change cannot be renewal")?;
                    }
                } else {
                    require(renewal_of.is_none(), "orphan renewal")?;
                }
                state
                    .latest
                    .insert(scope.clone(), review.candidate_id.clone());
                state.candidates.insert(
                    review.candidate_id.clone(),
                    CandidateState {
                        review,
                        scope,
                        stable_fact,
                        decision: None,
                        observations: Vec::new(),
                        renewal_of,
                    },
                );
            }
            Event::Observation {
                candidate_id,
                snapshot,
                observed_at,
            } => {
                let c = state
                    .candidates
                    .get_mut(&candidate_id)
                    .ok_or_else(|| audit("orphan observation"))?;
                let ids = identities(&snapshot)?;
                let key = acquisition(&snapshot)?;
                require(
                    row.kind == "Observation"
                        && row.candidate_id == candidate_id
                        && row.scope_key == c.scope
                        && row.revision == c.review.revision
                        && row.command_id == format!("observation:{candidate_id}:{key}")
                        && state.latest.get(&c.scope) == Some(&candidate_id)
                        && observed_at >= c.review.discovered_at
                        && ids.0 == c.scope
                        && ids.1 == c.stable_fact
                        && key != acquisition(&c.review.snapshot)?
                        && !c
                            .observations
                            .iter()
                            .any(|s| acquisition(s).ok().as_ref() == Some(&key)),
                    "invalid observation binding",
                )?;
                c.observations.push(snapshot);
            }
            Event::Decision { fact } => {
                let c = state
                    .candidates
                    .get_mut(&fact.candidate_id)
                    .ok_or_else(|| audit("orphan decision"))?;
                require(
                    row.kind == "Decision"
                        && row.candidate_id == fact.candidate_id
                        && row.scope_key == c.scope
                        && row.revision == c.review.revision
                        && row.command_id == format!("decision:{}", fact.candidate_id)
                        && c.decision.is_none()
                        && state.latest.get(&c.scope) == Some(&fact.candidate_id)
                        && fact.token == c.review.evidence_token
                        && fact.decided_at >= c.review.discovered_at
                        && fact.decided_at < c.review.expires_at
                        && !fact.operator.trim().is_empty()
                        && fact.operator.trim() == fact.operator
                        && !fact.reason.trim().is_empty()
                        && fact.reason.trim() == fact.reason,
                    "invalid decision",
                )?;
                if fact.decision == ReviewDecision::Confirm {
                    let receipt = legacy::exact_daily_change_confirmation_receipt_on_conn(
                        conn,
                        &c.review.snapshot.query,
                    )?
                    .ok_or_else(|| audit("confirmation missing"))?;
                    require(
                        fact.confirmation == Some((receipt.confirmation_id, receipt.record_hash)),
                        "confirmation link mismatch",
                    )?;
                } else {
                    require(
                        fact.confirmation.is_none(),
                        "reject cannot grant confirmation",
                    )?;
                }
                c.decision = Some(fact);
            }
        }
        state.seq = row.seq;
        state.head = row.record_hash;
    }
    #[derive(diesel::QueryableByName)]
    struct HighWater {
        #[diesel(sql_type=BigInt)]
        seq: i64,
    }
    let high: HighWater = diesel::sql_query("SELECT coalesce(max(seq),0) AS seq FROM sqlite_sequence WHERE name='daily_change_review_event'").get_result(conn)?;
    require(high.seq == state.seq, "review chain truncated")?;
    Ok(state)
}

fn append(
    conn: &mut SqliteConnection,
    state: &State,
    c: &CandidateReview,
    scope: &str,
    kind: &str,
    command: &str,
    event: &Event,
) -> ReviewResult<()> {
    let mut row = Row {
        seq: state.seq + 1,
        schema_version: 1,
        command_id: command.into(),
        scope_key: scope.into(),
        candidate_id: c.candidate_id.clone(),
        revision: c.revision,
        kind: kind.into(),
        payload: json(event)?,
        previous_hash: state.head.clone(),
        record_hash: String::new(),
    };
    row.record_hash = row_hash(&row)?;
    let n=diesel::sql_query("INSERT INTO daily_change_review_event(seq,schema_version,command_id,scope_key,candidate_id,revision,kind,payload,previous_hash,record_hash) VALUES (?,1,?,?,?,?,?,?,?,?)")
        .bind::<BigInt,_>(row.seq).bind::<Text,_>(row.command_id).bind::<Text,_>(row.scope_key).bind::<Text,_>(row.candidate_id)
        .bind::<BigInt,_>(row.revision).bind::<Text,_>(row.kind).bind::<Text,_>(row.payload).bind::<Text,_>(row.previous_hash).bind::<Text,_>(row.record_hash).execute(conn)?;
    require(n == 1, "event append did not affect exactly one row")
}
fn view(state: &State, id: &str, now: DateTime<Utc>) -> ReviewResult<CandidateReview> {
    let c = state.candidates.get(id).ok_or(ReviewError::NotFound)?;
    let mut result = c.review.clone();
    result.observations = c.observations.clone();
    result.status = if state.latest.get(&c.scope) != Some(&c.review.candidate_id) {
        "Superseded"
    } else if let Some(d) = &c.decision {
        if d.decision == ReviewDecision::Confirm {
            "Confirmed"
        } else {
            "Rejected"
        }
    } else if now >= c.review.expires_at {
        "Expired"
    } else {
        "Pending"
    }
    .into();
    Ok(result)
}

pub(crate) fn discover_on_conn(
    conn: &mut SqliteConnection,
    evidence: &crate::data_gateway::historical_bars::QualifiedDailyChangeDiscovery,
    now: DateTime<Utc>,
) -> ReviewResult<CandidateReview> {
    conn.immediate_transaction::<_, ReviewError, _>(|conn| {
        let state = load(conn)?;
        let snapshot = evidence.snapshot();
        let (scope, stable_fact) = identities(snapshot)?;
        let revision = if let Some(id) = state.latest.get(&scope) {
            let existing = &state.candidates[id];
            if existing.stable_fact == stable_fact {
                let key = acquisition(snapshot)?;
                for old in
                    std::iter::once(&existing.review.snapshot).chain(existing.observations.iter())
                {
                    if acquisition(old)? == key {
                        if old != snapshot {
                            return Err(ReviewError::Conflict);
                        }
                        return view(&state, id, now);
                    }
                }
                require(
                    now >= existing.review.discovered_at,
                    "observation clock regressed",
                )?;
                append(
                    conn,
                    &state,
                    &existing.review,
                    &scope,
                    "Observation",
                    &format!("observation:{id}:{key}"),
                    &Event::Observation {
                        candidate_id: id.clone(),
                        snapshot: snapshot.clone(),
                        observed_at: now,
                    },
                )?;
                return view(&load(conn)?, id, now);
            }
            require(
                now >= existing.review.discovered_at,
                "revision clock regressed",
            )?;
            existing.review.revision + 1
        } else {
            if legacy::exact_daily_change_confirmation_receipt_on_conn(conn, &snapshot.query)?
                .is_some()
            {
                return Err(ReviewError::AlreadyConfirmed);
            }
            1
        };
        let review = candidate(snapshot, revision, now)?;
        append(
            conn,
            &state,
            &review,
            &scope,
            "Candidate",
            &format!("candidate:{}", review.candidate_id),
            &Event::Candidate {
                review: review.clone(),
                scope: scope.clone(),
                stable_fact,
                renewal_of: None,
            },
        )?;
        Ok(review)
    })
}

pub fn review_on_conn(
    conn: &mut SqliteConnection,
    candidate: &str,
    now: DateTime<Utc>,
) -> ReviewResult<CandidateReview> {
    conn.transaction::<_, ReviewError, _>(|conn| view(&load(conn)?, candidate, now))
}

pub fn decide_on_conn(
    conn: &mut SqliteConnection,
    candidate: &str,
    token: &str,
    decision: ReviewDecision,
    operator: &str,
    reason: &str,
    now: DateTime<Utc>,
) -> ReviewResult<CandidateReview> {
    conn.immediate_transaction::<_, ReviewError, _>(|conn| {
        let state = load(conn)?;
        let c = state
            .candidates
            .get(candidate)
            .ok_or(ReviewError::NotFound)?;
        if token != c.review.evidence_token {
            return Err(ReviewError::InvalidToken);
        }
        if let Some(d) = &c.decision {
            if d.decision == decision && d.operator == operator && d.reason == reason {
                return view(&state, candidate, now);
            }
            return Err(ReviewError::Conflict);
        }
        if state.latest.get(&c.scope) != Some(&c.review.candidate_id) {
            return Err(ReviewError::Superseded);
        }
        if now >= c.review.expires_at {
            return Err(ReviewError::Expired);
        }
        require(
            now >= c.review.discovered_at
                && !operator.trim().is_empty()
                && operator.trim() == operator
                && !reason.trim().is_empty()
                && reason.trim() == reason,
            "invalid decision input",
        )?;
        let confirmation = if decision == ReviewDecision::Confirm {
            let receipt = match legacy::exact_daily_change_confirmation_receipt_on_conn(
                conn,
                &c.review.snapshot.query,
            )? {
                Some(receipt) => receipt,
                None => legacy::append_daily_change_confirmation_in_transaction(
                    conn,
                    &legacy::DailyChangeConfirmationInput {
                        query: c.review.snapshot.query.clone(),
                        operator_identity: operator.into(),
                        reason: reason.into(),
                        confirmed_at: now.fixed_offset(),
                    },
                )?,
            };
            Some((receipt.confirmation_id, receipt.record_hash))
        } else {
            None
        };
        let fact = DecisionFact {
            candidate_id: candidate.into(),
            token: token.into(),
            decision,
            operator: operator.into(),
            reason: reason.into(),
            decided_at: now,
            confirmation,
        };
        append(
            conn,
            &state,
            &c.review,
            &c.scope,
            "Decision",
            &format!("decision:{candidate}"),
            &Event::Decision { fact },
        )?;
        view(&load(conn)?, candidate, now)
    })
}

pub fn renew_on_conn(
    conn: &mut SqliteConnection,
    old_candidate: &str,
    token: &str,
    now: DateTime<Utc>,
) -> ReviewResult<CandidateReview> {
    conn.immediate_transaction::<_, ReviewError, _>(|conn| {
        let state = load(conn)?;
        let c = state
            .candidates
            .get(old_candidate)
            .ok_or(ReviewError::NotFound)?;
        if token != c.review.evidence_token {
            return Err(ReviewError::InvalidToken);
        }
        let latest = &state.latest[&c.scope];
        if latest != old_candidate {
            if state.candidates[latest].renewal_of.as_deref() == Some(old_candidate) {
                return view(&state, latest, now);
            }
            return Err(ReviewError::Superseded);
        }
        if c.decision.is_some() || now < c.review.expires_at {
            return Err(ReviewError::Conflict);
        }
        let review = candidate(&c.review.snapshot, c.review.revision + 1, now)?;
        append(
            conn,
            &state,
            &review,
            &c.scope,
            "Candidate",
            &format!("candidate:{}", review.candidate_id),
            &Event::Candidate {
                review: review.clone(),
                scope: c.scope.clone(),
                stable_fact: c.stable_fact.clone(),
                renewal_of: Some(old_candidate.into()),
            },
        )?;
        Ok(review)
    })
}

pub(crate) fn admit_on_conn(
    conn: &mut SqliteConnection,
    snapshot: &ReviewSnapshot,
) -> ReviewResult<bool> {
    conn.transaction::<_, ReviewError, _>(|conn| {
        let (scope, stable) = identities(snapshot)?;
        if !super::daily_change_review_schema_v1::is_present(conn)? {
            return Ok(legacy::has_exact_daily_change_confirmation_on_conn(
                conn,
                &snapshot.query,
            )?);
        }
        let state = load(conn)?;
        if let Some(id) = state.latest.get(&scope) {
            let c = &state.candidates[id];
            return Ok(c.stable_fact == stable
                && c.decision
                    .as_ref()
                    .is_some_and(|d| d.decision == ReviewDecision::Confirm));
        }
        Ok(legacy::has_exact_daily_change_confirmation_on_conn(
            conn,
            &snapshot.query,
        )?)
    })
}

#[cfg(test)]
pub(crate) fn install_owned_test_fixture(conn: &mut SqliteConnection) {
    legacy::create_schema(conn).unwrap();
    super::daily_change_review_schema_v1::create_schema(conn).unwrap();
}

#[cfg(test)]
#[path = "daily_change_review_tests.rs"]
mod tests;
