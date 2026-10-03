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
use crate::data_gateway::ordinary_daily_change_window::{
    self as window, CompleteWindowProof, PreparedWindowReceipt, QualifiedDailyChangeWindow,
    WindowStatus,
};
use crate::data_gateway::ordinary_daily_change_window_contract as window_contract;

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
    WindowObservation {
        window_id: String,
        request_identity: String,
        proof_identity: String,
        acquisition_identity: String,
        proof: CompleteWindowProof,
        receipt: PreparedWindowReceipt,
        observed_at: DateTime<Utc>,
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

#[derive(Clone)]
struct CandidateState {
    review: CandidateReview,
    scope: String,
    stable_fact: String,
    decision: Option<DecisionFact>,
    observations: Vec<ReviewSnapshot>,
    renewal_of: Option<String>,
}
#[derive(Default, Clone)]
struct State {
    candidates: BTreeMap<String, CandidateState>,
    windows: BTreeMap<String, (CompleteWindowProof, PreparedWindowReceipt)>,
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
    if snapshot.schema_version == 2 {
        let (_, _, stable) = window::snapshot_fact(snapshot).map_err(|e| audit(e.to_string()))?;
        return Ok((
            hash(&(
                "scope",
                &snapshot.instrument,
                snapshot.query.previous_date,
                snapshot.query.current_date,
            ))?,
            stable,
        ));
    }
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
    if snapshot.schema_version == 2 {
        let (_, a, _) = window::snapshot_fact(snapshot).map_err(|e| audit(e.to_string()))?;
        return Ok(window_contract::digest(
            b"BR171_ORDINARY_PAIR_ACQUISITION_V1\0",
            &window_contract::encode(&a, window_contract::PROOF_LIMIT)
                .map_err(|e| audit(e.to_string()))?,
        ));
    }
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

#[cfg(test)]
thread_local! {
    static EVENT_OWNED_DECODE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn decode_review_event(payload: &str) -> ReviewResult<Event> {
    if window_event_budget(payload.as_bytes())? {
        window_contract::preflight(payload.as_bytes(), window_contract::PROOF_LIMIT)
            .map_err(|e| audit(e.to_string()))?;
    }
    #[cfg(test)]
    EVENT_OWNED_DECODE_HITS.with(|hits| hits.set(hits.get() + 1));
    serde_json::from_str(payload).map_err(|e| audit(e.to_string()))
}

/// Structural discriminator only: fixed stack and fixed token scratch, no owned
/// JSON strings/tree. Opaque schema1 subtrees are scanned without WG07 limits.
/// serde subsequently validates grammar, duplicate fields and canonical bytes.
fn window_event_budget(bytes: &[u8]) -> ReviewResult<bool> {
    #[derive(Clone, Copy, PartialEq)]
    enum Name {
        Other,
        Kind,
        Review,
        Snapshot,
        Version,
        Candidate,
        Observation,
        Decision,
        Window,
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Place {
        Other,
        Root,
        Review,
        Snapshot,
    }
    #[derive(Clone, Copy)]
    struct Frame {
        object: bool,
        // 0: next key/array element or end; 1: colon; 2: member value;
        // 3: comma or end. Full JSON grammar remains serde's responsibility.
        state: u8,
        place: Place,
        key: Name,
    }
    fn string(bytes: &[u8], pos: &mut usize) -> ReviewResult<Name> {
        *pos += 1;
        let mut token = [0u8; 32];
        let mut count = 0usize;
        while *pos < bytes.len() {
            let mut ch = u32::from(bytes[*pos]);
            *pos += 1;
            if ch == u32::from(b'"') {
                return Ok(match token.get(..count) {
                    Some(b"kind") => Name::Kind,
                    Some(b"review") => Name::Review,
                    Some(b"snapshot") => Name::Snapshot,
                    Some(b"schema_version") => Name::Version,
                    Some(b"Candidate") => Name::Candidate,
                    Some(b"Observation") => Name::Observation,
                    Some(b"Decision") => Name::Decision,
                    Some(b"WindowObservation") => Name::Window,
                    _ => Name::Other,
                });
            }
            if ch == u32::from(b'\\') {
                let escape = *bytes
                    .get(*pos)
                    .ok_or_else(|| audit("event string escape"))?;
                *pos += 1;
                ch = match escape {
                    b'u' => {
                        let digits = bytes
                            .get(*pos..*pos + 4)
                            .ok_or_else(|| audit("event unicode escape"))?;
                        let mut code = 0u32;
                        for digit in digits {
                            code = code * 16
                                + char::from(*digit)
                                    .to_digit(16)
                                    .ok_or_else(|| audit("event unicode digit"))?;
                        }
                        *pos += 4;
                        code
                    }
                    b'"' | b'\\' | b'/' => u32::from(escape),
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => 10,
                    b'r' => 13,
                    b't' => 9,
                    _ => return Err(audit("event string escape")),
                };
            }
            if count < token.len() {
                token[count] = u8::try_from(ch).unwrap_or(0xff);
            }
            // Saturate beyond the fixed scratch; oversized opaque strings stay unallocated.
            count = (count + 1).min(token.len() + 1);
        }
        Err(audit("event unterminated string"))
    }
    let empty = Frame {
        object: false,
        state: 0,
        place: Place::Other,
        key: Name::Other,
    };
    let mut stack = [empty; 256];
    let mut depth = 0usize;
    let mut pos = 0usize;
    let mut root_seen = false;
    let mut kind = Name::Other;
    let mut kinds = 0usize;
    let mut versions = 0usize;
    let mut needs = false;
    while pos < bytes.len() {
        if bytes[pos].is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        if depth == 0 && root_seen {
            return Err(audit("event trailing JSON"));
        }
        if depth > 0 {
            let frame = &mut stack[depth - 1];
            if frame.state == 3 {
                if bytes[pos] == b',' {
                    frame.state = 0;
                    pos += 1;
                    continue;
                }
                if bytes[pos] != if frame.object { b'}' } else { b']' } {
                    return Err(audit("event JSON separator"));
                }
            }
            if bytes[pos] == if frame.object { b'}' } else { b']' } {
                if !matches!(frame.state, 0 | 3) {
                    return Err(audit("event incomplete member"));
                }
                depth -= 1;
                pos += 1;
                continue;
            }
            if frame.object && frame.state == 0 {
                if bytes[pos] != b'"' {
                    return Err(audit("event member key"));
                }
                frame.key = string(bytes, &mut pos)?;
                frame.state = 1;
                continue;
            }
            if frame.object && frame.state == 1 {
                if bytes[pos] != b':' {
                    return Err(audit("event member colon"));
                }
                frame.state = 2;
                pos += 1;
                continue;
            }
        }
        let (place, key) = if depth == 0 {
            (Place::Other, Name::Other)
        } else {
            let f = &mut stack[depth - 1];
            f.state = 3;
            (f.place, f.key)
        };
        let is_kind = place == Place::Root && key == Name::Kind;
        let is_version = place == Place::Snapshot && key == Name::Version;
        if is_kind {
            kinds += 1;
        }
        if is_version {
            versions += 1;
        }
        match bytes[pos] {
            b'{' | b'[' => {
                if is_kind || is_version {
                    needs = true;
                }
                if depth == stack.len() {
                    return Err(audit("event JSON nesting"));
                }
                let object = bytes[pos] == b'{';
                let child = if !root_seen {
                    Place::Root
                } else if object && place == Place::Root && key == Name::Review {
                    Place::Review
                } else if object
                    && matches!(place, Place::Root | Place::Review)
                    && key == Name::Snapshot
                {
                    Place::Snapshot
                } else {
                    Place::Other
                };
                if !root_seen && !object {
                    return Err(audit("event root object"));
                }
                root_seen = true;
                stack[depth] = Frame {
                    object,
                    state: 0,
                    place: child,
                    key: Name::Other,
                };
                depth += 1;
                pos += 1;
            }
            b'"' => {
                let value = string(bytes, &mut pos)?;
                if is_kind {
                    kind = value;
                }
                if is_version {
                    needs = true;
                }
            }
            _ => {
                let start = pos;
                while pos < bytes.len()
                    && !bytes[pos].is_ascii_whitespace()
                    && !matches!(bytes[pos], b',' | b'}' | b']')
                {
                    pos += 1;
                }
                if start == pos {
                    return Err(audit("event scalar token"));
                }
                if is_kind || (is_version && &bytes[start..pos] != b"1") {
                    needs = true;
                }
            }
        }
    }
    require(root_seen && depth == 0, "event incomplete JSON")?;
    Ok(needs
        || kinds != 1
        || kind == Name::Window
        || !matches!(kind, Name::Candidate | Name::Observation | Name::Decision)
        || (matches!(kind, Name::Candidate | Name::Observation) && versions != 1))
}

fn load(conn: &mut SqliteConnection) -> ReviewResult<State> {
    if !super::daily_change_review_schema_v1::is_present(conn)? {
        return Err(ReviewError::Unavailable);
    }
    #[derive(diesel::QueryableByName)]
    struct Oversized {
        #[diesel(sql_type=BigInt)]
        n: i64,
    }
    let oversized:Oversized=diesel::sql_query("SELECT count(*) AS n FROM daily_change_review_event WHERE length(CAST(payload AS BLOB))>8388608 OR length(CAST(command_id AS BLOB))>16384 OR length(CAST(scope_key AS BLOB))>16384 OR length(CAST(candidate_id AS BLOB))>16384 OR length(CAST(previous_hash AS BLOB))>16384 OR length(CAST(record_hash AS BLOB))>16384").get_result(conn)?;
    require(oversized.n == 0, "review row resource bound")?;
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
        require(
            row.payload.len() <= window_contract::PROOF_LIMIT,
            "review event byte limit",
        )?;
        let event = decode_review_event(&row.payload)?;
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
                if fact.decision == ReviewDecision::Confirm && c.review.snapshot.schema_version == 1
                {
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
            Event::WindowObservation {
                window_id,
                request_identity,
                proof_identity,
                acquisition_identity,
                proof,
                receipt,
                observed_at,
            } => {
                require(
                    row.kind == "Observation"
                        && row.revision == 1
                        && window_id == format!("br171_window_{acquisition_identity}")
                        && row.candidate_id == window_id
                        && row.scope_key == format!("window:{request_identity}")
                        && row.command_id == format!("window:{acquisition_identity}")
                        && observed_at == proof.frozen.invoked_at
                        && receipt.request_identity == request_identity
                        && receipt.proof_identity == proof_identity
                        && receipt.acquisition_identity == acquisition_identity,
                    "window row identity",
                )?;
                validate_window_receipt(&state, &proof, &receipt)?;
                require(
                    state
                        .windows
                        .insert(acquisition_identity, (proof, receipt))
                        .is_none(),
                    "duplicate window acquisition",
                )?;
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
    validate_window_closure(&state)?;
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
) -> ReviewResult<String> {
    append_identity(
        conn,
        state,
        &c.candidate_id,
        c.revision,
        scope,
        kind,
        command,
        event,
    )
}
fn append_identity(
    conn: &mut SqliteConnection,
    state: &State,
    candidate_id: &str,
    revision: i64,
    scope: &str,
    kind: &str,
    command: &str,
    event: &Event,
) -> ReviewResult<String> {
    let mut row = Row {
        seq: state.seq + 1,
        schema_version: 1,
        command_id: command.into(),
        scope_key: scope.into(),
        candidate_id: candidate_id.into(),
        revision,
        kind: kind.into(),
        payload: String::from_utf8(
            window_contract::encode(event, window_contract::PROOF_LIMIT)
                .map_err(|e| audit(e.to_string()))?,
        )
        .map_err(|e| audit(e.to_string()))?,
        previous_hash: state.head.clone(),
        record_hash: String::new(),
    };
    row.record_hash = row_hash(&row)?;
    let n=diesel::sql_query("INSERT INTO daily_change_review_event(seq,schema_version,command_id,scope_key,candidate_id,revision,kind,payload,previous_hash,record_hash) VALUES (?,1,?,?,?,?,?,?,?,?)")
        .bind::<BigInt,_>(row.seq).bind::<Text,_>(row.command_id).bind::<Text,_>(row.scope_key).bind::<Text,_>(row.candidate_id)
        .bind::<BigInt,_>(row.revision).bind::<Text,_>(row.kind).bind::<Text,_>(row.payload).bind::<Text,_>(row.previous_hash).bind::<Text,_>(&row.record_hash).execute(conn)?;
    require(n == 1, "event append did not affect exactly one row")?;
    Ok(row.record_hash)
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
        let mut state = load(conn)?;
        discover_in_transaction(conn, &mut state, evidence, now, true)
    })
}
fn discover_in_transaction(
    conn: &mut SqliteConnection,
    state: &mut State,
    evidence: &crate::data_gateway::historical_bars::QualifiedDailyChangeDiscovery,
    now: DateTime<Utc>,
    persist: bool,
) -> ReviewResult<CandidateReview> {
    let snapshot = evidence.snapshot();
    let (scope, stable_fact) = identities(snapshot)?;
    let revision = if let Some(id) = state.latest.get(&scope).cloned() {
        let existing = &state.candidates[&id];
        if existing.stable_fact == stable_fact {
            let key = acquisition(snapshot)?;
            for old in
                std::iter::once(&existing.review.snapshot).chain(existing.observations.iter())
            {
                if acquisition(old)? == key {
                    if old != snapshot {
                        return Err(ReviewError::Conflict);
                    }
                    return view(state, &id, now);
                }
            }
            require(
                now >= existing.review.discovered_at,
                "observation clock regressed",
            )?;
            let head = if persist {
                append(
                    conn,
                    state,
                    &existing.review,
                    &scope,
                    "Observation",
                    &format!("observation:{id}:{key}"),
                    &Event::Observation {
                        candidate_id: id.clone(),
                        snapshot: snapshot.clone(),
                        observed_at: now,
                    },
                )?
            } else {
                String::new()
            };
            state.head = head;
            state.seq += 1;
            state
                .candidates
                .get_mut(&id)
                .ok_or(ReviewError::NotFound)?
                .observations
                .push(snapshot.clone());
            return view(state, &id, now);
        }
        require(
            now >= existing.review.discovered_at,
            "revision clock regressed",
        )?;
        existing
            .review
            .revision
            .checked_add(1)
            .ok_or_else(|| audit("revision overflow"))?
    } else {
        if snapshot.schema_version == 1
            && legacy::exact_daily_change_confirmation_receipt_on_conn(conn, &snapshot.query)?
                .is_some()
        {
            return Err(ReviewError::AlreadyConfirmed);
        }
        1
    };
    let review = candidate(snapshot, revision, now)?;
    let head = if persist {
        append(
            conn,
            state,
            &review,
            &scope,
            "Candidate",
            &format!("candidate:{}", review.candidate_id),
            &Event::Candidate {
                review: review.clone(),
                scope: scope.clone(),
                stable_fact: stable_fact.clone(),
                renewal_of: None,
            },
        )?
    } else {
        String::new()
    };
    state.head = head;
    state.seq += 1;
    state
        .latest
        .insert(scope.clone(), review.candidate_id.clone());
    state.candidates.insert(
        review.candidate_id.clone(),
        CandidateState {
            review: review.clone(),
            scope,
            stable_fact,
            decision: None,
            observations: Vec::new(),
            renewal_of: None,
        },
    );
    Ok(review)
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
        let confirmation =
            if decision == ReviewDecision::Confirm && c.review.snapshot.schema_version == 1 {
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
            return if snapshot.schema_version == 2 {
                Ok(false)
            } else {
                Ok(legacy::has_exact_daily_change_confirmation_on_conn(
                    conn,
                    &snapshot.query,
                )?)
            };
        }
        let state = load(conn)?;
        if let Some(allowed) = admit_in_state(&state, &scope, &stable) {
            return Ok(allowed);
        }
        if snapshot.schema_version == 2 {
            Ok(false)
        } else {
            Ok(legacy::has_exact_daily_change_confirmation_on_conn(
                conn,
                &snapshot.query,
            )?)
        }
    })
}
fn admit_in_state(state: &State, scope: &str, stable: &str) -> Option<bool> {
    state.latest.get(scope).map(|id| {
        let c = &state.candidates[id];
        c.stable_fact == stable
            && c.decision
                .as_ref()
                .is_some_and(|d| d.decision == ReviewDecision::Confirm)
    })
}
pub(crate) fn require_window_store(conn: &mut SqliteConnection) -> ReviewResult<()> {
    conn.transaction::<_, ReviewError, _>(|conn| load(conn).map(|_| ()))
}
pub(crate) fn admit_window_on_conn(
    conn: &mut SqliteConnection,
    window: &QualifiedDailyChangeWindow,
) -> ReviewResult<Vec<String>> {
    conn.transaction::<_, ReviewError, _>(|conn| {
        let state = load(conn)?;
        let mut ids = Vec::new();
        for evidence in window.candidates() {
            let (scope, stable) = identities(evidence.snapshot())?;
            if admit_in_state(&state, &scope, &stable) != Some(true) {
                return Err(audit("manual_confirmation_required"));
            }
            ids.push(state.latest[&scope].clone());
        }
        Ok(ids)
    })
}
pub(crate) fn prepare_window_on_conn(
    conn: &mut SqliteConnection,
    window: &QualifiedDailyChangeWindow,
    now: DateTime<Utc>,
) -> ReviewResult<PreparedWindowReceipt> {
    prepare_window_transaction(conn, window, now, |_| Ok(()))
}
fn window_receipt(
    window: &QualifiedDailyChangeWindow,
    candidates: Vec<CandidateReview>,
) -> PreparedWindowReceipt {
    PreparedWindowReceipt {
        request_identity: window.request_identity().into(),
        proof_identity: window.proof_identity().into(),
        acquisition_identity: window.acquisition_identity().into(),
        window_status: if candidates.is_empty() {
            WindowStatus::NoChanges
        } else {
            WindowStatus::Candidates
        },
        candidates,
    }
}
fn window_event(window: &QualifiedDailyChangeWindow, receipt: &PreparedWindowReceipt) -> Event {
    Event::WindowObservation {
        window_id: format!("br171_window_{}", receipt.acquisition_identity),
        request_identity: receipt.request_identity.clone(),
        proof_identity: receipt.proof_identity.clone(),
        acquisition_identity: receipt.acquisition_identity.clone(),
        proof: window.proof().clone(),
        receipt: receipt.clone(),
        observed_at: window.proof().frozen.invoked_at,
    }
}
fn prepare_window_transaction(
    conn: &mut SqliteConnection,
    window: &QualifiedDailyChangeWindow,
    now: DateTime<Utc>,
    after_load: impl FnOnce(&mut SqliteConnection) -> ReviewResult<()>,
) -> ReviewResult<PreparedWindowReceipt> {
    conn.immediate_transaction::<_, ReviewError, _>(|conn| {
        let mut state = load(conn)?;
        if let Some((proof, receipt)) = state.windows.get(window.acquisition_identity()) {
            if proof != window.proof() {
                return Err(ReviewError::Conflict);
            }
            return Ok(receipt.clone());
        }
        require(
            now == window.proof().frozen.invoked_at,
            "window invocation clock mismatch",
        )?;
        // Reserve all new serialized event/snapshot/receipt bytes before any DB append.
        let mut planned = state.clone();
        let mut previews = Vec::new();
        let mut total = 0usize;
        for evidence in window.candidates() {
            total = total
                .checked_add(
                    window_contract::encode(evidence.snapshot(), window_contract::PROOF_LIMIT)
                        .map_err(|e| audit(e.to_string()))?
                        .len(),
                )
                .ok_or_else(|| audit("window total overflow"))?;
            require(
                total <= window_contract::PREPARE_LIMIT,
                "window snapshot budget",
            )?;
            previews.push(discover_in_transaction(
                conn,
                &mut planned,
                evidence,
                now,
                false,
            )?);
        }
        let expected = window_receipt(window, previews);
        total = total
            .checked_add(
                window_contract::encode(&expected, window_contract::PREPARE_LIMIT)
                    .map_err(|e| audit(e.to_string()))?
                    .len(),
            )
            .ok_or_else(|| audit("window total overflow"))?;
        require(
            total <= window_contract::PREPARE_LIMIT,
            "window snapshot/receipt budget",
        )?;
        let event_bytes = window_contract::encode(
            &window_event(window, &expected),
            window_contract::PROOF_LIMIT,
        )
        .map_err(|e| audit(e.to_string()))?;
        window_contract::preflight(&event_bytes, window_contract::PROOF_LIMIT)
            .map_err(|e| audit(e.to_string()))?;
        validate_window_receipt(&planned, window.proof(), &expected)?;
        after_load(conn)?;
        let mut candidates = Vec::new();
        for evidence in window.candidates() {
            candidates.push(discover_in_transaction(
                conn, &mut state, evidence, now, true,
            )?);
        }
        let receipt = window_receipt(window, candidates);
        require(receipt == expected, "window transaction receipt changed")?;
        let id = format!("br171_window_{}", receipt.acquisition_identity);
        let head = append_identity(
            conn,
            &state,
            &id,
            1,
            &format!("window:{}", receipt.request_identity),
            "Observation",
            &format!("window:{}", receipt.acquisition_identity),
            &window_event(window, &receipt),
        )?;
        state.head = head;
        state.seq += 1;
        state.windows.insert(
            receipt.acquisition_identity.clone(),
            (window.proof().clone(), receipt.clone()),
        );
        validate_window_closure(&state)?;
        Ok(receipt)
    })
}

fn validate_window_receipt(
    state: &State,
    proof: &CompleteWindowProof,
    receipt: &PreparedWindowReceipt,
) -> ReviewResult<()> {
    let ids = window::proof_identities(proof).map_err(|e| audit(e.to_string()))?;
    require(
        ids == (
            receipt.request_identity.clone(),
            receipt.proof_identity.clone(),
            receipt.acquisition_identity.clone(),
        ),
        "window proof identities",
    )?;
    let interpreted = window::inspect_proof(proof).map_err(|e| audit(e.to_string()))?;
    require(
        receipt.candidates.len() == interpreted.pairs.len()
            && receipt.window_status
                == if interpreted.pairs.is_empty() {
                    WindowStatus::NoChanges
                } else {
                    WindowStatus::Candidates
                },
        "window status/cardinality",
    )?;
    for (i, (view, pair)) in receipt
        .candidates
        .iter()
        .zip(&interpreted.pairs)
        .enumerate()
    {
        let c = state
            .candidates
            .get(&view.candidate_id)
            .ok_or_else(|| audit("window candidate absent"))?;
        require(
            *view == self::view(state, &view.candidate_id, proof.frozen.invoked_at)?,
            "window receipt current view mismatch",
        )?;
        require(
            view.revision == c.review.revision
                && view.evidence_token == c.review.evidence_token
                && view.discovered_at == c.review.discovered_at
                && view.expires_at == c.review.expires_at
                && view.snapshot == c.review.snapshot,
            "window candidate receipt",
        )?;
        let matching = std::iter::once(&c.review.snapshot)
            .chain(c.observations.iter())
            .find(|s| {
                window::snapshot_fact(s).is_ok_and(|(f, a, _)| {
                    f == *pair
                        && a.window_acquisition_identity == receipt.acquisition_identity
                        && a.proof_identity == receipt.proof_identity
                        && a.pair_index == i
                })
            });
        require(matching.is_some(), "window pair acquisition missing")?;
        let (_, a, _) =
            window::snapshot_fact(matching.unwrap()).map_err(|e| audit(e.to_string()))?;
        let refs: Vec<_> = interpreted
            .evidence
            .sessions
            .iter()
            .filter(|s| s.date >= pair.previous.date && s.date <= pair.current.date)
            .flat_map(|s| s.evidence_refs.clone())
            .collect();
        require(
            a.daily_batch_id == interpreted.evidence.source.batch_id && a.pair_evidence == refs,
            "pair native provenance",
        )?;
    }
    Ok(())
}
fn validate_window_closure(state: &State) -> ReviewResult<()> {
    // Cache plain reconstructed facts once per window; replay never seals authority.
    let mut recorded = BTreeMap::new();
    for c in state.candidates.values() {
        for s in std::iter::once(&c.review.snapshot).chain(c.observations.iter()) {
            if s.schema_version != 2 {
                continue;
            }
            let (_, a, _) = window::snapshot_fact(s).map_err(|e| audit(e.to_string()))?;
            let (proof, receipt) = state
                .windows
                .get(&a.window_acquisition_identity)
                .ok_or_else(|| audit("orphan window candidate"))?;
            if !recorded.contains_key(&a.window_acquisition_identity) {
                let facts = window::inspect_proof(proof).map_err(|e| audit(e.to_string()))?;
                recorded.insert(a.window_acquisition_identity.clone(), facts);
            }
            let expected = window::recorded_pair_snapshot(
                &recorded[&a.window_acquisition_identity],
                &receipt.proof_identity,
                &receipt.acquisition_identity,
                a.pair_index,
            )
            .map_err(|e| audit(e.to_string()))?;
            require(*s == expected, "window snapshot native provenance")?;
            require(
                receipt.proof_identity == a.proof_identity
                    && receipt.candidates.get(a.pair_index).is_some_and(|v| {
                        v.candidate_id == c.review.candidate_id
                            || renewal_descends_from(state, c, &v.candidate_id)
                    }),
                "window reference closure",
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn install_owned_test_fixture(conn: &mut SqliteConnection) {
    legacy::create_schema(conn).unwrap();
    super::daily_change_review_schema_v1::create_schema(conn).unwrap();
}

#[cfg(test)]
#[path = "daily_change_review_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "daily_change_review_window_tests.rs"]
mod window_tests;

fn renewal_descends_from(state: &State, candidate: &CandidateState, ancestor: &str) -> bool {
    let mut current = candidate;
    let mut remaining = state.candidates.len();
    while let Some(id) = &current.renewal_of {
        if remaining == 0 {
            return false;
        }
        remaining -= 1;
        if id == ancestor {
            return true;
        }
        let Some(parent) = state.candidates.get(id) else {
            return false;
        };
        current = parent;
    }
    false
}
