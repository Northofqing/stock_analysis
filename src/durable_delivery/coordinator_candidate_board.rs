//! P-05 card-level delivery observation from the counted authority.
//!
//! A candidate card can contain many stocks. This API makes no claim that a
//! Strong prediction sample belongs to an accepted card or to an individual
//! receipt; that link needs its own frozen producer-to-occurrence contract.

use super::{
    build_validated_terminal_evidence, load_decision, parse_envelope, sha256_hex, DecisionState,
    DeliveryEnvelope, DurableDeliveryCoordinator, DurableDeliveryError,
    FoundationTerminalDisposition, PushKind, Result, StoredDecision,
};
use crate::durable_delivery::model::{validate_business_date, CooldownScope, DeliverySubKind};
use chrono::{NaiveDate, NaiveTime};
use rusqlite::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum CandidateBoardCardTerminalV1 {
    Pending,
    Accepted,
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
}

/// A v1 source contains no prediction-row identity. A v2 source declares
/// ordered frozen IDs, but only the prediction DB can verify those rows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum CandidateBoardSourceLinkV1 {
    UnlinkedV1,
    DeclaredV2 {
        ordered_rows: Vec<CandidateBoardSourceRowV2>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateBoardSourceRowV2 {
    prediction_row_id: i64,
    code: String,
}

impl CandidateBoardSourceRowV2 {
    pub fn prediction_row_id(&self) -> i64 {
        self.prediction_row_id
    }

    pub fn code(&self) -> &str {
        &self.code
    }
}

/// Read-side source contract. Field order matches the prediction-DB freeze's
/// exact canonical bytes; the DB read here does not prove its row facts.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateBoardSourceV2 {
    schema: String,
    business_date: String,
    occurrence_identity: String,
    target_date: String,
    calendar_authority_hash: String,
    trading_dates: Vec<String>,
    rendered_sha256: String,
    ordered_rows: Vec<CandidateBoardSourceRowV2>,
}

/// One frozen P-05 counted decision, with terminal evidence checked in the
/// same SQLite read snapshot. Hashes and IDs are returned instead of content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateBoardCardObservationV1 {
    decision_identity: String,
    business_date: String,
    occurrence_identity: String,
    envelope_sha256: String,
    source_binding_sha256: String,
    rendered_content_sha256: String,
    decision_state: DecisionState,
    terminal: CandidateBoardCardTerminalV1,
    terminal_attempt_identity: Option<String>,
    disposition_identity: Option<String>,
    terminal_evidence_sha256: Option<String>,
    accepted_channel: Option<String>,
}

impl CandidateBoardCardObservationV1 {
    pub fn decision_identity(&self) -> &str {
        &self.decision_identity
    }

    pub fn business_date(&self) -> &str {
        &self.business_date
    }

    pub fn occurrence_identity(&self) -> &str {
        &self.occurrence_identity
    }

    pub fn envelope_sha256(&self) -> &str {
        &self.envelope_sha256
    }

    pub fn source_binding_sha256(&self) -> &str {
        &self.source_binding_sha256
    }

    pub fn rendered_content_sha256(&self) -> &str {
        &self.rendered_content_sha256
    }

    pub const fn decision_state(&self) -> DecisionState {
        self.decision_state
    }

    pub const fn terminal(&self) -> CandidateBoardCardTerminalV1 {
        self.terminal
    }

    pub fn terminal_attempt_identity(&self) -> Option<&str> {
        self.terminal_attempt_identity.as_deref()
    }

    pub fn disposition_identity(&self) -> Option<&str> {
        self.disposition_identity.as_deref()
    }

    pub fn terminal_evidence_sha256(&self) -> Option<&str> {
        self.terminal_evidence_sha256.as_deref()
    }

    pub fn accepted_channel(&self) -> Option<&str> {
        self.accepted_channel.as_deref()
    }

    /// Counts only a physically accepted card. Manual acceptance remains a
    /// separate, explicitly labelled observation.
    pub const fn is_authoritative_accepted_card(&self) -> bool {
        matches!(self.terminal, CandidateBoardCardTerminalV1::Accepted)
    }

    pub fn canonical_sha256(&self) -> Result<String> {
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }
}

/// The card-level observation plus its explicitly versioned source link.
/// `DeclaredV2` is a counted source declaration, not a cross-DB row check or
/// physical row delivery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateBoardCardObservationV2 {
    card: CandidateBoardCardObservationV1,
    source_link: CandidateBoardSourceLinkV1,
}

impl CandidateBoardCardObservationV2 {
    pub fn card(&self) -> &CandidateBoardCardObservationV1 {
        &self.card
    }

    pub fn source_link(&self) -> &CandidateBoardSourceLinkV1 {
        &self.source_link
    }

    pub fn canonical_sha256(&self) -> Result<String> {
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }
}

impl DurableDeliveryCoordinator {
    /// Read all P-05 card decisions for one business date without calling a
    /// provider or sink. The existing coordinator owns the attested SQLite
    /// connection; all selected rows and authority checks share one DEFERRED
    /// snapshot. This is a card denominator, never a per-stock denominator.
    pub fn candidate_board_card_observations_for_date(
        &self,
        business_date: &str,
    ) -> Result<Vec<CandidateBoardCardObservationV1>> {
        Ok(self
            .candidate_board_card_observations_with_source_for_date(business_date)?
            .into_iter()
            .map(|observation| observation.card)
            .collect())
    }

    /// Read v1 and v2 sources in one durable snapshot. Historical v1 remains
    /// Unlinked even when its occurrence and card hash match a later freeze.
    pub fn candidate_board_card_observations_with_source_for_date(
        &self,
        business_date: &str,
    ) -> Result<Vec<CandidateBoardCardObservationV2>> {
        validate_business_date(business_date)?;
        self.with_connection(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let identities = {
                let mut query = transaction.prepare(
                    "SELECT decision_identity FROM delivery_decisions
                     WHERE business_date=?1 AND push_kind=?2
                     ORDER BY decision_identity",
                )?;
                let rows = query.query_map(
                    params![business_date, PushKind::CandidateBoard.as_str()],
                    |row| row.get::<_, String>(0),
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut observations = Vec::with_capacity(identities.len());
            let mut occurrences = BTreeSet::new();
            for identity in identities {
                let stored = load_decision(&transaction, &identity)?
                    .ok_or_else(|| DurableDeliveryError::DecisionNotFound(identity.clone()))?;
                if sha256_hex(&stored.envelope_canonical) != stored.envelope_sha256 {
                    return Err(p05_mismatch("frozen envelope hash mismatch"));
                }
                let envelope = parse_envelope(&stored.envelope_canonical)?;
                if envelope.canonical_bytes()? != stored.envelope_canonical {
                    return Err(p05_mismatch("frozen envelope is not canonical"));
                }
                let source_link = validate_p05_card_binding(&stored, &envelope, business_date)?;
                if !occurrences.insert(envelope.schedule_occurrence_identity.clone()) {
                    return Err(p05_mismatch("multiple decisions own exact occurrence"));
                }
                let (terminal, attempt, disp_id, evidence_sha256, accepted_channel) = if matches!(
                    stored.state,
                    DecisionState::Delivered
                        | DecisionState::RejectedDurable
                        | DecisionState::UncertainManualReview
                        | DecisionState::ManualResolvedRejected
                ) {
                    let verified =
                        build_validated_terminal_evidence(&transaction, &stored, &envelope, None)?;
                    let terminal = match verified.disposition {
                        FoundationTerminalDisposition::Accepted => {
                            CandidateBoardCardTerminalV1::Accepted
                        }
                        FoundationTerminalDisposition::ManualAccepted => {
                            CandidateBoardCardTerminalV1::ManualAccepted
                        }
                        FoundationTerminalDisposition::Rejected => {
                            CandidateBoardCardTerminalV1::Rejected
                        }
                        FoundationTerminalDisposition::Uncertain => {
                            CandidateBoardCardTerminalV1::Uncertain
                        }
                        FoundationTerminalDisposition::ManualNotDelivered => {
                            CandidateBoardCardTerminalV1::ManualNotDelivered
                        }
                    };
                    (
                        terminal,
                        verified.attempt_id,
                        Some(verified.ref_id),
                        Some(verified.evidence_sha256),
                        verified.accepted_channel,
                    )
                } else {
                    (
                        CandidateBoardCardTerminalV1::Pending,
                        None,
                        None,
                        None,
                        None,
                    )
                };
                let card = CandidateBoardCardObservationV1 {
                    decision_identity: stored.decision_identity,
                    business_date: envelope.business_date,
                    occurrence_identity: envelope.schedule_occurrence_identity,
                    envelope_sha256: stored.envelope_sha256,
                    source_binding_sha256: envelope.source_binding_sha256,
                    rendered_content_sha256: envelope.rendered_content_sha256,
                    decision_state: stored.state,
                    terminal,
                    terminal_attempt_identity: attempt,
                    disposition_identity: disp_id,
                    terminal_evidence_sha256: evidence_sha256,
                    accepted_channel,
                };
                observations.push(CandidateBoardCardObservationV2 { card, source_link });
            }
            transaction.commit()?;
            Ok(observations)
        })
    }
}

fn p05_mismatch(detail: &'static str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("P-05 card observation {detail}"))
}

fn validate_p05_card_binding(
    stored: &StoredDecision,
    envelope: &DeliveryEnvelope,
    business_date: &str,
) -> Result<CandidateBoardSourceLinkV1> {
    if envelope.decision_identity != stored.decision_identity
        || envelope.business_date != business_date
        || stored.retry_authorized
        || stored.task_binding_present
    {
        return Err(p05_mismatch("decision/envelope binding mismatch"));
    }
    validate_candidate_board_source(envelope)
}

/// The counted store can validate source shape and exact bytes, but cannot
/// assert that v2 IDs still exist in the separate prediction database.
pub(super) fn validate_candidate_board_source(
    envelope: &DeliveryEnvelope,
) -> Result<CandidateBoardSourceLinkV1> {
    if envelope.push_kind != PushKind::CandidateBoard
        || envelope.sub_kind != DeliverySubKind::None
        || envelope.cooldown_scope != CooldownScope::Global
        || envelope.scope_key != "GLOBAL"
        || envelope.retry_authorized
        || envelope.task_binding.is_some()
        || envelope.foundation_binding().is_some()
        || envelope.provider_observed_at.is_some()
        || envelope.provider_as_of.is_some()
        || !envelope.original_batch_ids.is_empty()
    {
        return Err(p05_mismatch("decision/envelope binding mismatch"));
    }
    let business_date = envelope.business_date.as_str();
    let date = canonical_p05_date(business_date)?;
    let prefix = format!("candidate-board:{business_date}:");
    let hhmm = envelope
        .schedule_occurrence_identity
        .strip_prefix(&prefix)
        .ok_or_else(|| p05_mismatch("occurrence family mismatch"))?;
    let valid_time = hhmm.len() == 5
        && NaiveTime::parse_from_str(hhmm, "%H:%M")
            .is_ok_and(|time| time.format("%H:%M").to_string() == hhmm);
    if !valid_time {
        return Err(p05_mismatch("occurrence time mismatch"));
    }
    let source_hash = sha256_hex(&envelope.source_binding_canonical);
    if envelope.source_binding_sha256 != source_hash
        || envelope.source_evidence_fingerprint != source_hash
        || envelope.delivery_subject_hash != source_hash
    {
        return Err(p05_mismatch("source/subject digest mismatch"));
    }
    let source: serde_json::Value = serde_json::from_slice(&envelope.source_binding_canonical)?;
    let fields = source
        .as_object()
        .ok_or_else(|| p05_mismatch("source binding is not an object"))?;
    match fields.get("schema").and_then(serde_json::Value::as_str) {
        Some("candidate-board-v1") => {
            if fields.len() != 3
                || fields
                    .get("business_date")
                    .and_then(serde_json::Value::as_str)
                    != Some(business_date)
                || fields
                    .get("rendered_sha256")
                    .and_then(serde_json::Value::as_str)
                    != Some(envelope.rendered_content_sha256.as_str())
                || serde_json::to_vec(&source)? != envelope.source_binding_canonical
            {
                return Err(p05_mismatch("v1 source binding content mismatch"));
            }
            Ok(CandidateBoardSourceLinkV1::UnlinkedV1)
        }
        Some("candidate-board-v2") => {
            let frozen: CandidateBoardSourceV2 =
                serde_json::from_slice(&envelope.source_binding_canonical)?;
            if serde_json::to_vec(&frozen)? != envelope.source_binding_canonical
                || frozen.schema != "candidate-board-v2"
                || frozen.business_date != business_date
                || frozen.occurrence_identity != envelope.schedule_occurrence_identity
                || frozen.rendered_sha256 != envelope.rendered_content_sha256
                || !valid_sha256_text(&frozen.calendar_authority_hash)
                || std::str::from_utf8(&envelope.rendered_content).is_err()
                || frozen.trading_dates.len() != 6
                || frozen.trading_dates.first().map(String::as_str) != Some(business_date)
                || frozen.trading_dates.last().map(String::as_str)
                    != Some(frozen.target_date.as_str())
                || frozen.ordered_rows.is_empty()
            {
                return Err(p05_mismatch("v2 source binding content mismatch"));
            }
            if !crate::calendar::verified_a_share_trading_day(date)
                .map_err(|_| p05_mismatch("v2 trading calendar unavailable"))?
            {
                return Err(p05_mismatch("v2 business date is not a trading day"));
            }
            let mut expected = date;
            for date_text in frozen.trading_dates.iter().skip(1) {
                expected = crate::calendar::verified_next_a_share_trading_day(expected)
                    .map_err(|_| p05_mismatch("v2 trading calendar unavailable"))?;
                if canonical_p05_date(date_text)? != expected {
                    return Err(p05_mismatch("v2 trading dates mismatch"));
                }
            }
            let mut row_ids = BTreeSet::new();
            let mut codes = BTreeSet::new();
            for row in &frozen.ordered_rows {
                if row.prediction_row_id <= 0
                    || !row_ids.insert(row.prediction_row_id)
                    || row.code.is_empty()
                    || !row
                        .code
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                    || !codes.insert(row.code.as_str())
                {
                    return Err(p05_mismatch("v2 ordered row membership invalid"));
                }
            }
            Ok(CandidateBoardSourceLinkV1::DeclaredV2 {
                ordered_rows: frozen.ordered_rows,
            })
        }
        _ => Err(p05_mismatch("unknown source schema")),
    }
}

fn canonical_p05_date(value: &str) -> Result<NaiveDate> {
    let date = validate_business_date(value)?;
    if date.format("%Y-%m-%d").to_string() != value {
        return Err(p05_mismatch("source date is not canonical"));
    }
    Ok(date)
}

fn valid_sha256_text(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
