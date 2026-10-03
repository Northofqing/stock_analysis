//! Explicit-window WG07 owner. Only actual acquisition can mint qualification.
use super::{
    historical_bars::QualifiedDailyChangeDiscovery, ordinary_daily_change_window_contract as c,
};
use crate::{
    database::daily_change_review::{self as review, CandidateReview, ReviewSnapshot},
    grpc_client::{
        client::external_historical_read::{ExternalHistoricalReadClient, WindowTransportEvidence},
        connection_qualification::ConnectionIdentity,
        external_v1::build_ordinary_window_query,
    },
    market_domain::InstrumentId,
};
pub use c::{OrdinaryDailyBar, SourceDecimal};
use chrono::{DateTime, NaiveDate, Utc};
use diesel::{connection::SimpleConnection, Connection, SqliteConnection};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OrdinaryDailyChangeWindowRequest {
    pub instrument: InstrumentId,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub as_of: DateTime<Utc>,
    pub source_profile: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FailureKind {
    InvalidRequest,
    UnsupportedProfileOrVersion,
    CalendarUnavailableOrInapplicable,
    IncompleteSession,
    ConnectionQualification,
    CapabilityUnavailable,
    TransportFailure,
    EnvelopeRejected,
    RequestBindingMismatch,
    NativeIdentityMissingOrMismatch,
    AdjustmentMissingOrMismatch,
    UnknownSourceTerminal,
    IncompleteRange,
    DuplicateOrUnexpectedSession,
    SourceRejected,
    PublicationMissingOrAfterAsOf,
    RevisionMissingOrConflict,
    CorrectionProofMissing,
    LifecycleCoverageMissing,
    InvalidBar,
    AuditFailure,
    PersistenceFailure,
}
#[derive(Debug, Clone, Serialize)]
pub struct FailureItem {
    pub instrument: Option<InstrumentId>,
    pub date: Option<NaiveDate>,
    pub range: Option<(NaiveDate, NaiveDate)>,
    pub kind: FailureKind,
    pub retryable: bool,
    pub evidence_refs: Vec<String>,
    pub reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CommitOutcome {
    Unknown,
}
#[derive(Debug, Serialize)]
pub struct DiscoveryFailure {
    pub(crate) stage: String,
    pub(crate) commit_outcome: Option<CommitOutcome>,
    pub(crate) request_identity: Option<String>,
    pub(crate) failures: Vec<FailureItem>,
    pub(crate) retained_evidence: Option<WindowTransportEvidence>,
    pub(crate) retained_window_proof: Option<CompleteWindowProof>,
}
impl DiscoveryFailure {
    pub(crate) fn one(kind: FailureKind, why: &str) -> Self {
        Self::from_items(vec![FailureItem {
            instrument: None,
            date: None,
            range: None,
            kind,
            retryable: false,
            evidence_refs: Vec::new(),
            reason: why.into(),
        }])
    }
    pub(crate) fn from_items(failures: Vec<FailureItem>) -> Self {
        assert!(!failures.is_empty());
        Self {
            stage: "qualification".into(),
            commit_outcome: None,
            request_identity: None,
            failures,
            retained_evidence: None,
            retained_window_proof: None,
        }
    }
    fn context_input(mut self, request: &OrdinaryDailyChangeWindowRequest) -> Self {
        for item in &mut self.failures {
            if item.instrument.is_none() {
                item.instrument = Some(request.instrument.clone());
            }
            if item.date.is_none() {
                item.range = Some((request.from, request.to));
            }
        }
        self
    }
    fn with_window(mut self, window: &QualifiedDailyChangeWindow) -> Self {
        self.request_identity = Some(window.request_identity().into());
        self.retained_window_proof = Some(window.proof().clone());
        let r = &window.proof().frozen.request;
        for item in &mut self.failures {
            if item.instrument.is_none() {
                item.instrument = Some(r.instrument.clone());
            }
            if item.date.is_none() {
                item.range = Some((r.from, r.to));
            }
        }
        self
    }
    pub fn request_identity(&self) -> Option<&str> {
        self.request_identity.as_deref()
    }
    pub fn commit_outcome(&self) -> Option<CommitOutcome> {
        self.commit_outcome
    }
    pub fn stage(&self) -> &str {
        &self.stage
    }
    pub fn failures(&self) -> &[FailureItem] {
        &self.failures
    }
    fn with_retryable(mut self, retryable: Option<bool>) -> Self {
        for item in &mut self.failures {
            item.retryable = retryable.unwrap_or(false);
        }
        self
    }
    fn retained(mut self, request: &str, evidence: &WindowTransportEvidence) -> Self {
        self.request_identity = Some(request.into());
        self.retained_evidence = Some(evidence.clone());
        self
    }
}
impl std::fmt::Display for DiscoveryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WG07 {}: {:?}", self.stage, self.failures)
    }
}
impl std::error::Error for DiscoveryFailure {}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowStatus {
    Candidates,
    NoChanges,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWindowReceipt {
    pub request_identity: String,
    pub proof_identity: String,
    pub acquisition_identity: String,
    pub window_status: WindowStatus,
    pub candidates: Vec<CandidateReview>,
}
#[derive(Debug)]
pub struct AdmittedOrdinaryDailyChangeWindow {
    request: c::FrozenRequest,
    request_identity: String,
    proof_identity: String,
    bars: Vec<OrdinaryDailyBar>,
    accepted_candidate_ids: Vec<String>,
}
impl AdmittedOrdinaryDailyChangeWindow {
    pub fn instrument(&self) -> &InstrumentId {
        &self.request.request.instrument
    }
    pub fn from(&self) -> NaiveDate {
        self.request.request.from
    }
    pub fn to(&self) -> NaiveDate {
        self.request.request.to
    }
    pub fn as_of(&self) -> DateTime<Utc> {
        self.request.request.as_of
    }
    pub fn bars(&self) -> &[OrdinaryDailyBar] {
        &self.bars
    }
    pub fn request_identity(&self) -> &str {
        &self.request_identity
    }
    pub fn proof_identity(&self) -> &str {
        &self.proof_identity
    }
    pub fn accepted_candidate_ids(&self) -> &[String] {
        &self.accepted_candidate_ids
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompleteWindowProof {
    pub frozen: c::FrozenRequest,
    pub connection: ConnectionIdentity,
    pub health_hex: String,
    pub capabilities_hex: String,
    pub request_hex: String,
    pub response_hex: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairAcquisition {
    pub window_acquisition_identity: String,
    pub proof_identity: String,
    pub pair_index: usize,
    pub daily_batch_id: String,
    pub pair_evidence: Vec<c::EvidenceRef>,
}
pub(crate) struct QualifiedPair {
    snapshot: ReviewSnapshot,
}
impl QualifiedPair {
    pub(crate) fn into_snapshot(self) -> ReviewSnapshot {
        self.snapshot
    }
}
pub(crate) struct QualifiedDailyChangeWindow {
    proof: CompleteWindowProof,
    request_identity: String,
    proof_identity: String,
    acquisition_identity: String,
    candidates: Vec<QualifiedDailyChangeDiscovery>,
    bars: Vec<OrdinaryDailyBar>,
}
impl QualifiedDailyChangeWindow {
    pub(crate) fn proof(&self) -> &CompleteWindowProof {
        &self.proof
    }
    pub(crate) fn request_identity(&self) -> &str {
        &self.request_identity
    }
    pub(crate) fn proof_identity(&self) -> &str {
        &self.proof_identity
    }
    pub(crate) fn acquisition_identity(&self) -> &str {
        &self.acquisition_identity
    }
    pub(crate) fn candidates(&self) -> &[QualifiedDailyChangeDiscovery] {
        &self.candidates
    }
}
fn unhex(s: &str, limit: usize) -> c::Result<Vec<u8>> {
    if s.len() % 2 != 0 || s.len() / 2 > limit {
        return Err(c::failure(
            FailureKind::EnvelopeRejected,
            "proof hex length",
        ));
    }
    hex::decode(s).map_err(|_| c::failure(FailureKind::EnvelopeRejected, "proof hex"))
}
pub(crate) fn proof_identities(proof: &CompleteWindowProof) -> c::Result<(String, String, String)> {
    let req = unhex(&proof.request_hex, c::MIB)?;
    let request = c::digest(b"BR171_ORDINARY_WINDOW_REQUEST_V1\0", &req);
    let bytes = c::encode(proof, c::PROOF_LIMIT)?;
    let proof_id = c::digest(b"BR171_ORDINARY_WINDOW_PROOF_V1\0", &bytes);
    let acquisition = c::digest(b"BR171_ORDINARY_WINDOW_ACQUISITION_V1\0", &bytes);
    Ok((request, proof_id, acquisition))
}
/// Reconstruct recorded facts without minting live transport authority. Used by
/// strict ledger replay; it does not call public acquisition or perform I/O.
pub(crate) fn inspect_proof(proof: &CompleteWindowProof) -> c::Result<c::Interpreted> {
    let r = &proof.frozen.request;
    let p = c::profile(&format!(
        "{}@{}",
        r.source_profile_id, r.source_profile_version
    ))?;
    let fresh = c::freeze(
        OrdinaryDailyChangeWindowRequest {
            instrument: r.instrument.clone(),
            from: r.from,
            to: r.to,
            as_of: r.as_of,
            source_profile: format!("{}@{}", r.source_profile_id, r.source_profile_version),
        },
        proof.frozen.invoked_at,
        &p,
    )?;
    if fresh != proof.frozen || !proof.connection.validate_recorded() {
        return Err(c::failure(
            FailureKind::ConnectionQualification,
            "recorded calendar/connection",
        ));
    }
    let decoder = crate::grpc_client::external_decoder::ExternalDecoder::for_descriptor(
        &proof.connection.descriptor_sha256,
    )
    .map_err(|_| c::failure(FailureKind::ConnectionQualification, "descriptor"))?;
    let health = decoder
        .health(&unhex(&proof.health_hex, c::PROOF_LIMIT)?)
        .map_err(|_| c::failure(FailureKind::ConnectionQualification, "health bytes"))?;
    let trust = crate::grpc_client::build_identity::BuildIdentityTrust::bundled()
        .map_err(|_| c::failure(FailureKind::ConnectionQualification, "recorded trust"))?;
    trust
        .recorded_health(
            &proof.connection.policy_sha256,
            &proof.connection.descriptor_sha256,
            &health,
        )
        .map_err(|_| {
            c::failure(
                FailureKind::ConnectionQualification,
                "recorded Health identity",
            )
        })?;
    let caps = decoder
        .capabilities(&unhex(&proof.capabilities_hex, c::PROOF_LIMIT)?)
        .map_err(|_| c::failure(FailureKind::CapabilityUnavailable, "capabilities bytes"))?;
    crate::grpc_client::client::external_historical_read::require_window_capability(
        &caps.capabilities,
        &p,
    )
    .map_err(|_| c::failure(FailureKind::CapabilityUnavailable, "exact capability"))?;
    let request_bytes = unhex(&proof.request_hex, c::MIB)?;
    decoder
        .query_request(&request_bytes)
        .map_err(|_| c::failure(FailureKind::RequestBindingMismatch, "request wire"))?;
    let request = crate::grpc_client::external_pb::magic::market::v1::QueryRequest::decode(
        request_bytes.as_slice(),
    )
    .map_err(|_| c::failure(FailureKind::RequestBindingMismatch, "request protobuf"))?;
    if !request.context.as_ref().is_some_and(|context| {
        context.protocol_version == 1
            && !context.request_id.is_empty()
            && context.request_id.len() <= 16384
    }) {
        return Err(c::failure(
            FailureKind::RequestBindingMismatch,
            "request context",
        ));
    }
    let payload = request
        .payload
        .as_ref()
        .ok_or_else(|| c::failure(FailureKind::RequestBindingMismatch, "request payload"))?;
    if request.allow_unadmitted
        || request.preferred_provider != p.provider
        || payload.schema != c::REQUEST_SCHEMA
        || payload.schema_version != 1
        || payload.content_type != "application/json; charset=utf-8"
        || c::decode::<c::RequestV1>(&payload.data, c::MIB)? != *r
    {
        return Err(c::failure(
            FailureKind::RequestBindingMismatch,
            "request contract",
        ));
    }
    let raw = unhex(&proof.response_hex, c::PROOF_LIMIT)?;
    crate::grpc_client::external_query_transport::admit_external_payload(&raw)
        .map_err(|_| c::failure(FailureKind::EnvelopeRejected, "response wire"))?;
    let response = decoder
        .query(&raw)
        .map_err(|_| c::failure(FailureKind::EnvelopeRejected, "response protobuf"))?;
    use crate::grpc_client::external_pb::magic::market::v1::{AdmissionState, Operation};
    if response.request_id
        != request
            .context
            .as_ref()
            .map(|v| v.request_id.as_str())
            .unwrap_or("")
        || response.operation != Operation::HistoricalBars as i32
        || response.admission != AdmissionState::Admitted as i32
        || !response.complete
        || response.selected_provider != p.provider
        || !response.diagnostic_blocker.is_empty()
        || response.records.len() != 1
    {
        return Err(c::failure(
            FailureKind::EnvelopeRejected,
            "response envelope",
        ));
    }
    let record = &response.records[0];
    if record.schema != c::RESULT_SCHEMA
        || record.schema_version != 1
        || record.content_type != "application/json; charset=utf-8"
    {
        return Err(c::failure(
            FailureKind::EnvelopeRejected,
            "result family/version",
        ));
    }
    let interpreted = c::interpret(&record.data, &proof.frozen, &request_bytes, &p)?;
    if response.batch_id != interpreted.evidence.source.batch_id
        || response.observed_at.parse::<DateTime<Utc>>().ok()
            != Some(interpreted.evidence.source.observed_at)
        || match &interpreted.evidence.source.source_at {
            c::SourceAt::NotProvided => !response.source_at.is_empty(),
            c::SourceAt::Present(at) => {
                response.source_at.parse::<DateTime<Utc>>().ok() != Some(*at)
            }
        }
    {
        return Err(c::failure(
            FailureKind::EnvelopeRejected,
            "source/envelope acquisition mismatch",
        ));
    }
    Ok(interpreted)
}
pub(crate) fn snapshot_for(
    pair: &c::PairFact,
    acquisition: &PairAcquisition,
) -> c::Result<ReviewSnapshot> {
    use crate::database::daily_change_confirmation::DailyChangeConfirmationQuery;
    let action = if pair.actions.is_empty() {
        None
    } else {
        Some(c::digest(
            b"BR171_ORDINARY_ACTIONS_V1\0",
            &c::encode(&pair.actions, c::PROOF_LIMIT)?,
        ))
    };
    Ok(ReviewSnapshot {
        schema_version: 2,
        discovery_contract: "ordinary-full-window-v1".into(),
        rule_version: pair.rule.clone(),
        instrument: pair.instrument.clone(),
        query: DailyChangeConfirmationQuery {
            code: pair.instrument.code().into(),
            previous_date: pair.previous.date,
            current_date: pair.current.date,
            previous_close: pair.previous.close.value.clone(),
            current_close: pair.current.close.value.clone(),
            calculated_pct: c::percent(pair)?,
            daily_provider: pair.provider.clone(),
            daily_source: pair.source.clone(),
            daily_batch_id: acquisition.daily_batch_id.clone(),
            lifecycle_provider: pair.provider.clone(),
            lifecycle_batch_id: acquisition.daily_batch_id.clone(),
            listing_date: Some(pair.listing_date),
            corporate_action_identity: action,
        },
        fact_payload: serde_json::from_slice(&c::encode(pair, c::PROOF_LIMIT)?)
            .map_err(|_| c::failure(FailureKind::AuditFailure, "pair serialization"))?,
        raw_evidence: serde_json::from_slice(&c::encode(acquisition, c::PROOF_LIMIT)?)
            .map_err(|_| c::failure(FailureKind::AuditFailure, "acquisition serialization"))?,
    })
}
pub(crate) fn snapshot_fact(
    snapshot: &ReviewSnapshot,
) -> c::Result<(c::PairFact, PairAcquisition, String)> {
    if snapshot.schema_version != 2 || snapshot.discovery_contract != "ordinary-full-window-v1" {
        return Err(c::failure(FailureKind::AuditFailure, "snapshot contract"));
    }
    let pair: c::PairFact = c::decode(
        &c::encode(&snapshot.fact_payload, c::PROOF_LIMIT)?,
        c::PROOF_LIMIT,
    )?;
    pair.previous
        .validate(&pair.instrument, pair.previous.date)?;
    pair.current.validate(&pair.instrument, pair.current.date)?;
    let acquisition: PairAcquisition = c::decode(
        &c::encode(&snapshot.raw_evidence, c::PROOF_LIMIT)?,
        c::PROOF_LIMIT,
    )?;
    if snapshot_for(&pair, &acquisition)? != *snapshot
        || !c::anomalous(&pair.previous, &pair.current)?
        || pair.rule != "br171-close-change-v1"
        || pair.previous.date >= pair.current.date
        || pair.instrument != snapshot.instrument
    {
        return Err(c::failure(
            FailureKind::AuditFailure,
            "snapshot reconstruction",
        ));
    }
    let stable = c::digest(
        b"BR171_ORDINARY_WINDOW_FACT_V1\0",
        &c::encode(&pair, c::PROOF_LIMIT)?,
    );
    Ok((pair, acquisition, stable))
}
fn qualify(proof: CompleteWindowProof) -> c::Result<QualifiedDailyChangeWindow> {
    let interpreted = inspect_proof(&proof)?;
    let (request_identity, proof_identity, acquisition_identity) = proof_identities(&proof)?;
    let mut candidates = Vec::new();
    let mut budget = c::encode(&proof, c::PROOF_LIMIT)?.len();
    for (pair_index, pair) in interpreted.pairs.iter().enumerate() {
        let refs = interpreted
            .evidence
            .sessions
            .iter()
            .filter(|s| s.date >= pair.previous.date && s.date <= pair.current.date)
            .flat_map(|s| s.evidence_refs.clone())
            .collect();
        let a = PairAcquisition {
            window_acquisition_identity: acquisition_identity.clone(),
            proof_identity: proof_identity.clone(),
            pair_index,
            daily_batch_id: interpreted.evidence.source.batch_id.clone(),
            pair_evidence: refs,
        };
        let snapshot = snapshot_for(pair, &a)?;
        budget = budget
            .checked_add(c::encode(&snapshot, c::PROOF_LIMIT)?.len())
            .filter(|n| *n <= c::PREPARE_LIMIT)
            .ok_or_else(|| {
                c::failure(FailureKind::EnvelopeRejected, "candidate snapshots budget")
            })?;
        candidates.push(QualifiedDailyChangeDiscovery::from_window_pair(
            QualifiedPair { snapshot },
        ));
    }
    Ok(QualifiedDailyChangeWindow {
        proof,
        request_identity,
        proof_identity,
        acquisition_identity,
        candidates,
        bars: interpreted.bars,
    })
}
pub(crate) fn open_existing(path: &Path, read_only: bool) -> c::Result<SqliteConnection> {
    let fail = |e: String| c::failure(FailureKind::PersistenceFailure, &e);
    let path = path.canonicalize().map_err(|e| fail(e.to_string()))?;
    if !path.is_file() {
        return Err(fail("existing database required".into()));
    }
    let mut uri = url::Url::from_file_path(path).map_err(|_| fail("database path".into()))?;
    uri.set_query(Some(if read_only { "mode=ro" } else { "mode=rw" }));
    let mut conn = SqliteConnection::establish(uri.as_str()).map_err(|e| fail(e.to_string()))?;
    conn.batch_execute("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .map_err(|e| fail(e.to_string()))?;
    Ok(conn)
}
async fn acquire(
    input: OrdinaryDailyChangeWindowRequest,
    bundle: &Path,
    now: DateTime<Utc>,
) -> c::Result<QualifiedDailyChangeWindow> {
    acquire_inner(input.clone(), bundle, now)
        .await
        .map_err(|e| e.context_input(&input))
}
async fn acquire_inner(
    input: OrdinaryDailyChangeWindowRequest,
    bundle: &Path,
    now: DateTime<Utc>,
) -> c::Result<QualifiedDailyChangeWindow> {
    let p = c::profile(&input.source_profile)?;
    let frozen = c::freeze(input, now, &p)?;
    let query = build_ordinary_window_query(&frozen, &p)?;
    let request_bytes = query.wire_bytes()?;
    let request_identity = c::digest(b"BR171_ORDINARY_WINDOW_REQUEST_V1\0", &request_bytes);
    let mut retained = WindowTransportEvidence::default();
    let mut reader = ExternalHistoricalReadClient::connect_client_bundle(bundle)
        .await
        .map_err(|e| {
            c::failure(FailureKind::ConnectionQualification, &e.to_string())
                .with_retryable(e.details().retryable)
                .retained(&request_identity, &retained)
        })?;
    let observation = reader
        .query_window(query, &p, &mut retained)
        .await
        .map_err(|e| {
            let kind = if retained.stage == "Capabilities" {
                FailureKind::CapabilityUnavailable
            } else {
                FailureKind::ConnectionQualification
            };
            c::failure(kind, &e.to_string())
                .with_retryable(e.details().retryable)
                .retained(&request_identity, &retained)
        })?;
    if let Err(e) = &observation.result {
        return Err(c::failure(
            if observation.status.is_some() {
                FailureKind::TransportFailure
            } else {
                FailureKind::EnvelopeRejected
            },
            &e.to_string(),
        )
        .with_retryable(e.details().retryable)
        .retained(&request_identity, &retained));
    }
    if observation.request_bytes != request_bytes {
        return Err(c::failure(
            FailureKind::RequestBindingMismatch,
            "transport issued bytes",
        )
        .retained(&request_identity, &retained));
    }
    let raw = observation.wire.payload().ok_or_else(|| {
        c::failure(FailureKind::EnvelopeRejected, "response wire absent")
            .retained(&request_identity, &retained)
    })?;
    let hex_bytes = raw
        .len()
        .checked_add(request_bytes.len())
        .and_then(|n| n.checked_add(observation.health_wire.len()))
        .and_then(|n| n.checked_add(observation.capabilities_wire.len()))
        .and_then(|n| n.checked_mul(2))
        .filter(|n| *n <= c::PROOF_LIMIT)
        .ok_or_else(|| {
            c::failure(FailureKind::EnvelopeRejected, "complete proof byte budget")
                .retained(&request_identity, &retained)
        })?;
    let _ = hex_bytes;
    let proof = CompleteWindowProof {
        frozen,
        connection: observation.connection_identity,
        health_hex: hex::encode(observation.health_wire),
        capabilities_hex: hex::encode(observation.capabilities_wire),
        request_hex: hex::encode(request_bytes),
        response_hex: hex::encode(raw),
    };
    qualify(proof).map_err(|e| e.retained(&request_identity, &retained))
}
pub async fn prepare_window(
    request: OrdinaryDailyChangeWindowRequest,
    client_bundle: &Path,
    database: &Path,
) -> c::Result<PreparedWindowReceipt> {
    prepare_at(request, client_bundle, database, Utc::now()).await
}
async fn prepare_at(
    request: OrdinaryDailyChangeWindowRequest,
    client_bundle: &Path,
    database: &Path,
    now: DateTime<Utc>,
) -> c::Result<PreparedWindowReceipt> {
    // Validate the profile before filesystem/network access, then acquire only
    // after an existing review namespace has been proved locally.
    c::profile(&request.source_profile)?;
    let mut conn = open_existing(database, false)?;
    review::require_window_store(&mut conn).map_err(review_failure)?;
    let qualified = acquire(request, client_bundle, now).await?;
    review::prepare_window_on_conn(&mut conn, &qualified, now)
        .map_err(|e| review_failure(e).with_window(&qualified))
}
pub async fn consume_window(
    request: OrdinaryDailyChangeWindowRequest,
    client_bundle: &Path,
    database: &Path,
) -> c::Result<AdmittedOrdinaryDailyChangeWindow> {
    consume_at(request, client_bundle, database, Utc::now()).await
}
async fn consume_at(
    request: OrdinaryDailyChangeWindowRequest,
    client_bundle: &Path,
    database: &Path,
    now: DateTime<Utc>,
) -> c::Result<AdmittedOrdinaryDailyChangeWindow> {
    c::profile(&request.source_profile)?;
    let mut conn = open_existing(database, true)?;
    review::require_window_store(&mut conn).map_err(review_failure)?;
    let q = acquire(request, client_bundle, now).await?;
    let ids = review::admit_window_on_conn(&mut conn, &q)
        .map_err(|e| review_failure(e).with_window(&q))?;
    Ok(AdmittedOrdinaryDailyChangeWindow {
        request: q.proof.frozen,
        request_identity: q.request_identity,
        proof_identity: q.proof_identity,
        bars: q.bars,
        accepted_candidate_ids: ids,
    })
}
fn review_failure(e: review::ReviewError) -> DiscoveryFailure {
    let database = matches!(e, review::ReviewError::Database(_));
    let mut failure = c::failure(
        if database {
            FailureKind::PersistenceFailure
        } else {
            FailureKind::AuditFailure
        },
        &e.to_string(),
    );
    // Diesel exposes a transaction error rather than a durable commit receipt.
    // Never retry an uncertain write from this owner.
    if database {
        failure.commit_outcome = Some(CommitOutcome::Unknown);
    }
    failure
}

#[cfg(test)]
#[path = "ordinary_daily_change_window_tests.rs"]
pub(crate) mod tests;
